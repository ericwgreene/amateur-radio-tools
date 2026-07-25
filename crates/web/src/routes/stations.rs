//! The station roster: every unique callsign this operator has ever heard.
//!
//! This is the long-lived view — a logbook tells you who you *worked*, this
//! tells you who is actually out there and how often you hear them. One row per
//! callsign, no matter how many times it has come up.

use actix_web::{HttpRequest, HttpResponse, get, post, web};
use askama::Template;
use askama_web::WebTemplate;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::Deserialize;

use crate::auth::session::{AuthedUser, SessionUser};
use crate::error::AppError;
use crate::routes::{AUTO_REFRESH_SECS, auto_refresh_cookie, auto_refresh_pref, dash};
use crate::services::observations::{self as obs_service, PromoteDetails};
use crate::services::stations as station_service;
use crate::state::AppState;
use crate::tools::callsign;
use entity::{observations, sessions, stations};

/// Most rows one page shows. The roster grows slowly (one row per *distinct*
/// station), so this is generous rather than a real constraint.
const ROSTER_LIMIT: u64 = 500;

/// A roster row, pre-formatted for the template.
pub struct StationView {
    pub callsign: String,
    pub name: String,
    pub qth: String,
    pub country: String,
    pub first_heard: String,
    pub last_heard: String,
    pub times_heard: i64,
    pub times_worked: i64,
    /// Whether this is the operator's own callsign, so it can be marked rather
    /// than looking like just another station.
    pub is_me: bool,
}

#[derive(Debug, Deserialize)]
pub struct RosterQuery {
    pub q: Option<String>,
    /// `last_heard` (default), `times_heard`, `first_heard`, or `callsign`.
    pub order: Option<String>,
    /// Set by the auto-refresh toggle link; absent on an ordinary visit.
    pub auto: Option<String>,
}

/// The sort orders the roster offers, and their labels.
///
/// Rendered from here rather than hardcoded in the template so the active one can
/// be marked `selected` — otherwise the dropdown resets to "Last heard" on every
/// reload while the rows stay sorted some other way.
pub const ORDERS: [(&str, &str); 4] = [
    ("last_heard", "Last heard"),
    ("times_heard", "Times heard"),
    ("first_heard", "First heard"),
    ("callsign", "Callsign"),
];

/// One entry in the sort dropdown.
pub struct OrderOption {
    pub value: &'static str,
    pub label: &'static str,
    pub selected: bool,
}

fn order_options(active: &str) -> Vec<OrderOption> {
    ORDERS
        .iter()
        .map(|(value, label)| OrderOption {
            value,
            label,
            selected: *value == active,
        })
        .collect()
}

/// The order actually in force, defaulting when the query says nothing usable.
fn active_order(query: &RosterQuery) -> String {
    match query.order.as_deref() {
        Some(o) if ORDERS.iter().any(|(v, _)| *v == o) => o.to_string(),
        _ => ORDERS[0].0.to_string(),
    }
}

async fn load_roster(
    state: &AppState,
    user_id: i64,
    own_callsign: Option<&str>,
    query: &RosterQuery,
) -> Result<Vec<StationView>, AppError> {
    let mut find = stations::Entity::find().filter(stations::Column::UserId.eq(user_id));
    if let Some(q) = &query.q {
        let q = q.trim();
        if !q.is_empty() {
            find = find.filter(stations::Column::Callsign.starts_with(callsign::normalize(q)));
        }
    }
    let find = match query.order.as_deref() {
        Some("times_heard") => find.order_by_desc(stations::Column::TimesHeard),
        Some("first_heard") => find.order_by_desc(stations::Column::FirstHeardAt),
        Some("callsign") => find.order_by_asc(stations::Column::Callsign),
        _ => find.order_by_desc(stations::Column::LastHeardAt),
    };
    let rows = find.limit(ROSTER_LIMIT).all(&state.db).await?;

    // "Times worked" is counted from the logbook rather than stored, so it can't
    // drift away from the contacts that back it.
    let calls: Vec<String> = rows.iter().map(|s| s.callsign.clone()).collect();
    let worked = station_service::worked_counts(&state.db, user_id, &calls).await?;

    Ok(rows
        .into_iter()
        .map(|s| StationView {
            is_me: own_callsign.is_some_and(|c| c.eq_ignore_ascii_case(&s.callsign)),
            times_worked: worked.get(&s.callsign).copied().unwrap_or(0),
            first_heard: s.first_heard_at.format("%Y-%m-%d").to_string(),
            last_heard: s.last_heard_at.format("%Y-%m-%d %H:%M").to_string(),
            times_heard: s.times_heard,
            name: dash(s.name),
            qth: dash(s.qth),
            country: dash(s.country),
            callsign: s.callsign,
        })
        .collect())
}

#[derive(Template, WebTemplate)]
#[template(path = "stations.html")]
struct StationsPage {
    current_user: Option<SessionUser>,
    stations: Vec<StationView>,
    query: String,
    orders: Vec<OrderOption>,
    /// Drives whether the tbody carries a polling trigger at all.
    auto_refresh: bool,
    refresh_secs: u32,
}

/// Fragment: just the table body, swapped in by the search box.
#[derive(Template, WebTemplate)]
#[template(path = "partials/station_rows.html")]
struct StationRows {
    stations: Vec<StationView>,
}

#[get("/stations")]
pub async fn stations_page(
    req: HttpRequest,
    user: AuthedUser,
    state: web::Data<AppState>,
    query: web::Query<RosterQuery>,
) -> Result<HttpResponse, AppError> {
    let own = own_callsign(&state, user.0.id).await?;
    let stations = load_roster(&state, user.0.id, own.as_deref(), &query).await?;
    let auto_refresh = auto_refresh_pref(&req, query.auto.as_deref());

    let page = StationsPage {
        query: query.q.clone().unwrap_or_default(),
        // The sort and search live inside the controls form, so the browser
        // resubmits their current values when the toggle is pressed — nothing
        // needs to be threaded into a URL here.
        orders: order_options(&active_order(&query)),
        current_user: Some(user.0),
        stations,
        auto_refresh,
        refresh_secs: AUTO_REFRESH_SECS,
    };
    // Rendered by hand rather than returned as a template, because the response
    // needs a Set-Cookie header when the toggle was used.
    let body = page.render().map_err(anyhow::Error::new)?;

    let mut response = HttpResponse::Ok();
    response.content_type("text/html; charset=utf-8");
    // Only write the cookie when the toggle was actually used, so an ordinary
    // visit doesn't set one for a preference the operator never expressed.
    if query.auto.is_some() {
        response.cookie(auto_refresh_cookie(
            auto_refresh,
            state.config.cookie_secure,
        ));
    }
    Ok(response.body(body))
}

/// HTMX: the roster body alone, for live search and re-sorting.
#[get("/stations/rows")]
pub async fn stations_rows(
    user: AuthedUser,
    state: web::Data<AppState>,
    query: web::Query<RosterQuery>,
) -> Result<StationRows, AppError> {
    let own = own_callsign(&state, user.0.id).await?;
    let stations = load_roster(&state, user.0.id, own.as_deref(), &query).await?;
    Ok(StationRows { stations })
}

/// One hearing, for the per-station history.
pub struct HearingView {
    pub id: i64,
    pub heard_at: String,
    pub band: String,
    pub mode: String,
    pub frequency: String,
    pub duration: String,
    pub transcript: String,
    pub session_label: String,
    pub session_id: i64,
    pub promoted: bool,
}

#[derive(Template, WebTemplate)]
#[template(path = "station_detail.html")]
struct StationDetailPage {
    current_user: Option<SessionUser>,
    callsign: String,
    name: String,
    qth: String,
    grid: String,
    country: String,
    first_heard: String,
    last_heard: String,
    times_heard: i64,
    times_worked: i64,
    hearings: Vec<HearingView>,
}

#[get("/stations/{callsign}")]
pub async fn station_detail(
    user: AuthedUser,
    state: web::Data<AppState>,
    path: web::Path<String>,
) -> Result<StationDetailPage, AppError> {
    let call = callsign::normalize(&path.into_inner());
    let station = stations::Entity::find()
        .filter(stations::Column::UserId.eq(user.0.id))
        .filter(stations::Column::Callsign.eq(&call))
        .one(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;

    let hearings = load_hearings(&state, user.0.id, &call).await?;
    let worked =
        station_service::worked_counts(&state.db, user.0.id, std::slice::from_ref(&call)).await?;

    Ok(StationDetailPage {
        current_user: Some(user.0),
        times_worked: worked.get(&call).copied().unwrap_or(0),
        first_heard: station.first_heard_at.format("%Y-%m-%d %H:%M").to_string(),
        last_heard: station.last_heard_at.format("%Y-%m-%d %H:%M").to_string(),
        times_heard: station.times_heard,
        name: dash(station.name),
        qth: dash(station.qth),
        grid: dash(station.grid),
        country: dash(station.country),
        callsign: call,
        hearings,
    })
}

async fn load_hearings(
    state: &AppState,
    user_id: i64,
    call: &str,
) -> Result<Vec<HearingView>, AppError> {
    let rows = observations::Entity::find()
        .filter(observations::Column::UserId.eq(user_id))
        .filter(observations::Column::Callsign.eq(call))
        .order_by_desc(observations::Column::HeardAt)
        .limit(ROSTER_LIMIT)
        .all(&state.db)
        .await?;

    // Label each hearing with the session it belongs to, fetched in one query
    // rather than one per row.
    let session_ids: Vec<i64> = rows.iter().map(|o| o.session_id).collect();
    let sessions: std::collections::HashMap<i64, sessions::Model> = if session_ids.is_empty() {
        Default::default()
    } else {
        sessions::Entity::find()
            .filter(sessions::Column::UserId.eq(user_id))
            .filter(sessions::Column::Id.is_in(session_ids))
            .all(&state.db)
            .await?
            .into_iter()
            .map(|s| (s.id, s))
            .collect()
    };

    Ok(rows.into_iter().map(|o| to_hearing(o, &sessions)).collect())
}

fn to_hearing(
    o: observations::Model,
    sessions: &std::collections::HashMap<i64, sessions::Model>,
) -> HearingView {
    let session_label = sessions
        .get(&o.session_id)
        .map(session_title)
        .unwrap_or_else(|| "—".to_string());
    HearingView {
        id: o.id,
        heard_at: o.heard_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        band: dash(o.band),
        mode: dash(o.mode),
        frequency: o
            .frequency_mhz
            .map(|f| format!("{f:.3}"))
            .unwrap_or_else(|| "—".to_string()),
        duration: o
            .duration_secs
            .map(|d| format!("{d:.1}s"))
            .unwrap_or_else(|| "—".to_string()),
        // Usually absent: uploading speech is opt-in, so most rows have nothing
        // here and the column stays quiet.
        transcript: dash(o.transcript),
        session_label,
        session_id: o.session_id,
        promoted: o.promoted_contact_id.is_some(),
    }
}

/// A human label for a session: its own if it has one, else kind + date.
pub fn session_title(s: &sessions::Model) -> String {
    match &s.label {
        Some(l) if !l.trim().is_empty() => l.clone(),
        _ => format!("{} {}", s.kind, s.started_at.format("%Y-%m-%d")),
    }
}

/// `POST /observations/{id}/promote` — log a heard station as a worked contact.
///
/// The bridge between the two halves of the site: hearing a station and working
/// it are different events, so they live in different tables, and this is what
/// turns one into the other once the QSO actually happens.
#[post("/observations/{id}/promote")]
pub async fn promote(
    user: AuthedUser,
    state: web::Data<AppState>,
    path: web::Path<i64>,
) -> Result<StationHearings, AppError> {
    let id = path.into_inner();
    // Promoting an already-promoted hearing is a no-op that still re-renders, so
    // a double-click is harmless rather than an error in the operator's face.
    let promotion = obs_service::promote(
        &state.db,
        user.0.id,
        id,
        PromoteDetails {
            notes: Some("Promoted from a monitored transmission.".to_string()),
            ..Default::default()
        },
    )
    .await?
    .ok_or(AppError::NotFound)?;

    let hearings = load_hearings(&state, user.0.id, &promotion.contact.callsign).await?;
    Ok(StationHearings { hearings })
}

/// Fragment: the per-station hearing rows, returned after a promote.
#[derive(Template, WebTemplate)]
#[template(path = "partials/hearing_rows.html")]
pub struct StationHearings {
    hearings: Vec<HearingView>,
}

/// The signed-in operator's own callsign, if they've set one.
async fn own_callsign(state: &AppState, user_id: i64) -> Result<Option<String>, AppError> {
    Ok(entity::users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await?
        .and_then(|u| u.callsign))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn session(label: Option<&str>) -> sessions::Model {
        sessions::Model {
            id: 1,
            user_id: 1,
            client_key: "k".into(),
            kind: "net".into(),
            label: label.map(str::to_string),
            started_at: Utc.with_ymd_and_hms(2026, 7, 24, 23, 0, 0).unwrap(),
            ended_at: None,
            band: None,
            mode: None,
            frequency_mhz: None,
            operator_callsign: None,
            grid: None,
            source: None,
            notes: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn session_title_prefers_the_label() {
        assert_eq!(
            session_title(&session(Some("Tuesday ARES"))),
            "Tuesday ARES"
        );
    }

    #[test]
    fn session_title_falls_back_to_kind_and_date() {
        assert_eq!(session_title(&session(None)), "net 2026-07-24");
        assert_eq!(
            session_title(&session(Some("   "))),
            "net 2026-07-24",
            "a blank label is not a label"
        );
    }
}
