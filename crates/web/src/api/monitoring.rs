//! REST API for monitoring logs: sessions, observations, and the station roster.
//!
//! This is what a receive-only monitor talks to. Two design choices run through the
//! whole module and are worth stating once:
//!
//! * **Sessions are addressed by the client's own key, never by a server id.** That is
//!   what lets a desktop app open a session while offline, record for hours, and upload
//!   later — it never has to round-trip to learn an identifier.
//! * **Ingest is a batch with partial success.** See
//!   [`crate::services::observations`] for why one bad row must not fail the request.
//!
//! Authentication is the same personal API token used by the contacts endpoints:
//! `Authorization: Bearer art_...`.

use actix_web::{HttpResponse, get, patch, post, web};
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};

use crate::api::error::ApiError;
use crate::api::{Page, Paged};
use crate::auth::api_token::ApiUser;
use crate::services::observations::{self as obs_service, MAX_BATCH, ObservationInput};
use crate::services::sessions::{self as session_service, SessionInput};
use crate::services::stations as station_service;
use crate::state::AppState;
use crate::tools::callsign;
use entity::{contacts, observations, sessions, stations};

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// Reject a kind the client made up, rather than silently storing it — an unrecognized
/// kind would quietly disappear from every filtered view.
fn check_kind(kind: &Option<String>) -> Result<(), ApiError> {
    match kind {
        Some(k) if !session_service::is_valid_kind(k) => Err(ApiError::BadRequest(format!(
            "kind must be one of {}",
            session_service::KINDS.join(", ")
        ))),
        _ => Ok(()),
    }
}

fn check_client_key(key: &str) -> Result<(), ApiError> {
    if key.trim().is_empty() {
        return Err(ApiError::BadRequest("client_key is required".to_string()));
    }
    Ok(())
}

/// `POST /api/v1/sessions` — open a session (idempotent on `client_key`).
///
/// Returns `201` the first time and `200` on a replay, so a client retrying after a
/// timeout can tell whether its earlier attempt actually landed.
#[post("/api/v1/sessions")]
pub async fn create_session(
    user: ApiUser,
    state: web::Data<AppState>,
    payload: web::Json<SessionInput>,
) -> Result<HttpResponse, ApiError> {
    let input = payload.into_inner();
    check_client_key(&input.client_key)?;
    check_kind(&input.kind)?;

    let result = session_service::upsert_by_client_key(&state.db, user.user.id, &input).await?;
    Ok(if result.created {
        HttpResponse::Created().json(result.session)
    } else {
        HttpResponse::Ok().json(result.session)
    })
}

/// `PATCH /api/v1/sessions/by-key/{client_key}` — relabel or close a session.
///
/// Addressed by client key rather than id, and it *creates* an unknown key instead of
/// 404ing. That upsert behaviour is deliberate: it makes the client's upload queue
/// order-independent, so a "session ended" message that reaches the server before its
/// "session opened" partner still produces the right row.
#[patch("/api/v1/sessions/by-key/{client_key}")]
pub async fn update_session(
    user: ApiUser,
    state: web::Data<AppState>,
    path: web::Path<String>,
    payload: web::Json<SessionInput>,
) -> Result<HttpResponse, ApiError> {
    let client_key = path.into_inner();
    check_client_key(&client_key)?;

    let mut input = payload.into_inner();
    check_kind(&input.kind)?;
    // The path is authoritative; a mismatched body field would otherwise silently
    // retarget the write.
    input.client_key = client_key;

    let result = session_service::upsert_by_client_key(&state.db, user.user.id, &input).await?;
    Ok(HttpResponse::Ok().json(result.session))
}

#[derive(Debug, Deserialize)]
pub struct SessionFilter {
    pub kind: Option<String>,
    /// `true` restricts to sessions still running (no end time recorded).
    pub open: Option<bool>,
}

/// `GET /api/v1/sessions` — list the caller's sessions, newest first.
#[get("/api/v1/sessions")]
pub async fn list_sessions(
    user: ApiUser,
    state: web::Data<AppState>,
    page: web::Query<Page>,
    filter: web::Query<SessionFilter>,
) -> Result<HttpResponse, ApiError> {
    let (limit, offset) = page.resolve();

    let mut query = sessions::Entity::find().filter(sessions::Column::UserId.eq(user.user.id));
    if let Some(kind) = &filter.kind {
        query = query.filter(sessions::Column::Kind.eq(kind));
    }
    if filter.open == Some(true) {
        query = query.filter(sessions::Column::EndedAt.is_null());
    }

    let total = query.clone().count(&state.db).await?;
    let items = query
        .order_by_desc(sessions::Column::StartedAt)
        .limit(limit)
        .offset(offset)
        .all(&state.db)
        .await?;

    Ok(HttpResponse::Ok().json(Paged {
        items,
        total,
        limit,
        offset,
    }))
}

/// A session plus the operator identity actually in force for it.
#[derive(Debug, Serialize)]
pub struct SessionDetail {
    #[serde(flatten)]
    pub session: sessions::Model,
    /// The session's own callsign if it set one, otherwise the operator's default from
    /// their profile. Resolved here so a client doesn't have to fetch both and apply the
    /// precedence rule itself.
    pub effective_callsign: Option<String>,
    pub effective_grid: Option<String>,
}

/// `GET /api/v1/sessions/{id}` — one session owned by the caller.
#[get("/api/v1/sessions/{id}")]
pub async fn get_session(
    user: ApiUser,
    state: web::Data<AppState>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let session = sessions::Entity::find_by_id(path.into_inner())
        .filter(sessions::Column::UserId.eq(user.user.id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;

    let (effective_callsign, effective_grid) =
        session_service::effective_operator(&session, &user.user);

    Ok(HttpResponse::Ok().json(SessionDetail {
        session,
        effective_callsign,
        effective_grid,
    }))
}

// ---------------------------------------------------------------------------
// Observations
// ---------------------------------------------------------------------------

/// A batch of hearings plus the session they belong to.
///
/// The session travels with every batch rather than being referenced by id. It costs a
/// couple of hundred bytes and removes all ordering constraints between the client's
/// queued session record and its queued observations.
#[derive(Debug, Deserialize)]
pub struct ObservationBatch {
    pub session: SessionInput,
    pub observations: Vec<ObservationInput>,
}

/// `POST /api/v1/observations` — batch ingest.
///
/// Responds `200`, not `201`: this is a partial-success operation, and the body reports
/// what was accepted, what was a replay, and what was rejected outright.
#[post("/api/v1/observations")]
pub async fn ingest_observations(
    user: ApiUser,
    state: web::Data<AppState>,
    payload: web::Json<ObservationBatch>,
) -> Result<HttpResponse, ApiError> {
    let batch = payload.into_inner();
    check_client_key(&batch.session.client_key)?;
    check_kind(&batch.session.kind)?;

    if batch.observations.len() > MAX_BATCH {
        return Err(ApiError::BadRequest(format!(
            "a batch may carry at most {MAX_BATCH} observations, got {}",
            batch.observations.len()
        )));
    }

    let outcome =
        obs_service::ingest_batch(&state.db, user.user.id, &batch.session, batch.observations)
            .await?;

    Ok(HttpResponse::Ok().json(outcome))
}

#[derive(Debug, Deserialize)]
pub struct ObservationFilter {
    pub session_id: Option<i64>,
    pub callsign: Option<String>,
    pub since: Option<DateTime<Utc>>,
}

/// `GET /api/v1/observations` — the monitoring log, newest first.
#[get("/api/v1/observations")]
pub async fn list_observations(
    user: ApiUser,
    state: web::Data<AppState>,
    page: web::Query<Page>,
    filter: web::Query<ObservationFilter>,
) -> Result<HttpResponse, ApiError> {
    let (limit, offset) = page.resolve();

    let mut query =
        observations::Entity::find().filter(observations::Column::UserId.eq(user.user.id));
    if let Some(session_id) = filter.session_id {
        query = query.filter(observations::Column::SessionId.eq(session_id));
    }
    if let Some(call) = &filter.callsign {
        query = query.filter(observations::Column::Callsign.eq(callsign::normalize(call)));
    }
    if let Some(since) = filter.since {
        query = query.filter(observations::Column::HeardAt.gte(since));
    }

    let total = query.clone().count(&state.db).await?;
    let items = query
        .order_by_desc(observations::Column::HeardAt)
        .limit(limit)
        .offset(offset)
        .all(&state.db)
        .await?;

    Ok(HttpResponse::Ok().json(Paged {
        items,
        total,
        limit,
        offset,
    }))
}

/// Optional QSO details supplied when promoting a hearing into a logbook contact.
#[derive(Debug, Default, Deserialize)]
pub struct PromoteInput {
    pub rst_sent: Option<String>,
    pub rst_received: Option<String>,
    pub worked_at: Option<DateTime<Utc>>,
    pub band: Option<String>,
    pub mode: Option<String>,
    pub frequency_mhz: Option<f64>,
    pub notes: Option<String>,
}

/// `POST /api/v1/observations/{id}/promote` — turn a hearing into a logbook contact.
///
/// Hearing a station and working it are different events, which is why observations and
/// contacts are separate tables. This is the bridge: once you actually make the QSO,
/// the observation you already logged becomes the seed for the logbook entry.
///
/// Promoting twice returns `200` with the contact created the first time rather than a
/// conflict — a client retrying a request whose response it never saw should converge,
/// not error.
#[post("/api/v1/observations/{id}/promote")]
pub async fn promote_observation(
    user: ApiUser,
    state: web::Data<AppState>,
    path: web::Path<i64>,
    payload: Option<web::Json<PromoteInput>>,
) -> Result<HttpResponse, ApiError> {
    let input = payload.map(|p| p.into_inner()).unwrap_or_default();
    let observation = observations::Entity::find_by_id(path.into_inner())
        .filter(observations::Column::UserId.eq(user.user.id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;

    if let Some(existing_id) = observation.promoted_contact_id {
        let existing = contacts::Entity::find_by_id(existing_id)
            .filter(contacts::Column::UserId.eq(user.user.id))
            .one(&state.db)
            .await?
            .ok_or(ApiError::NotFound)?;
        return Ok(HttpResponse::Ok().json(existing));
    }

    // Carry over whatever the station rollup already knows, so a promoted contact isn't
    // emptier than the roster row it came from.
    let station = stations::Entity::find()
        .filter(stations::Column::UserId.eq(user.user.id))
        .filter(stations::Column::Callsign.eq(&observation.callsign))
        .one(&state.db)
        .await?;

    let now = Utc::now();
    let contact = contacts::ActiveModel {
        user_id: Set(user.user.id),
        callsign: Set(observation.callsign.clone()),
        worked_at: Set(input.worked_at.unwrap_or(observation.heard_at)),
        band: Set(input.band.or_else(|| observation.band.clone())),
        mode: Set(input.mode.or_else(|| observation.mode.clone())),
        frequency_mhz: Set(input.frequency_mhz.or(observation.frequency_mhz)),
        rst_sent: Set(input.rst_sent),
        rst_received: Set(input.rst_received),
        grid: Set(station.as_ref().and_then(|s| s.grid.clone())),
        name: Set(station.as_ref().and_then(|s| s.name.clone())),
        qth: Set(station.as_ref().and_then(|s| s.qth.clone())),
        country: Set(observation.country.clone()),
        notes: Set(input.notes),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&state.db)
    .await?;

    let mut active = observation.into_active_model();
    active.promoted_contact_id = Set(Some(contact.id));
    active.update(&state.db).await?;

    Ok(HttpResponse::Created().json(contact))
}

// ---------------------------------------------------------------------------
// Stations
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct StationFilter {
    /// Case-insensitive callsign prefix search.
    pub q: Option<String>,
    /// `last_heard` (default), `first_heard`, `times_heard`, or `callsign`.
    pub order: Option<String>,
}

/// A roster row: the stored rollup plus the derived worked count.
#[derive(Debug, Serialize)]
pub struct StationRow {
    #[serde(flatten)]
    pub station: stations::Model,
    /// Counted from `contacts` at read time rather than stored — see
    /// [`crate::services::stations::worked_counts`].
    pub times_worked: i64,
}

/// `GET /api/v1/stations` — the unique-station roster.
///
/// This is the "log of unique contacts over time": one row per station ever heard.
#[get("/api/v1/stations")]
pub async fn list_stations(
    user: ApiUser,
    state: web::Data<AppState>,
    page: web::Query<Page>,
    filter: web::Query<StationFilter>,
) -> Result<HttpResponse, ApiError> {
    let (limit, offset) = page.resolve();

    let mut query = stations::Entity::find().filter(stations::Column::UserId.eq(user.user.id));
    if let Some(q) = &filter.q {
        let q = q.trim();
        if !q.is_empty() {
            query = query.filter(stations::Column::Callsign.starts_with(callsign::normalize(q)));
        }
    }

    let total = query.clone().count(&state.db).await?;
    let query = match filter.order.as_deref() {
        Some("first_heard") => query.order_by_desc(stations::Column::FirstHeardAt),
        Some("times_heard") => query.order_by_desc(stations::Column::TimesHeard),
        Some("callsign") => query.order_by_asc(stations::Column::Callsign),
        _ => query.order_by_desc(stations::Column::LastHeardAt),
    };
    let rows = query.limit(limit).offset(offset).all(&state.db).await?;

    Ok(HttpResponse::Ok().json(Paged {
        items: with_worked_counts(&state.db, user.user.id, rows).await?,
        total,
        limit,
        offset,
    }))
}

/// `GET /api/v1/stations/{callsign}` — one station from the roster.
#[get("/api/v1/stations/{callsign}")]
pub async fn get_station(
    user: ApiUser,
    state: web::Data<AppState>,
    path: web::Path<String>,
) -> Result<HttpResponse, ApiError> {
    let call = callsign::normalize(&path.into_inner());
    let station = stations::Entity::find()
        .filter(stations::Column::UserId.eq(user.user.id))
        .filter(stations::Column::Callsign.eq(&call))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;

    let rows = with_worked_counts(&state.db, user.user.id, vec![station]).await?;
    Ok(HttpResponse::Ok().json(rows.into_iter().next().ok_or(ApiError::NotFound)?))
}

/// Attach derived worked counts to a page of stations in one extra query.
async fn with_worked_counts(
    db: &sea_orm::DatabaseConnection,
    user_id: i64,
    rows: Vec<stations::Model>,
) -> Result<Vec<StationRow>, sea_orm::DbErr> {
    let calls: Vec<String> = rows.iter().map(|s| s.callsign.clone()).collect();
    let counts = station_service::worked_counts(db, user_id, &calls).await?;
    Ok(rows
        .into_iter()
        .map(|station| {
            let times_worked = counts.get(&station.callsign).copied().unwrap_or(0);
            StationRow {
                station,
                times_worked,
            }
        })
        .collect())
}
