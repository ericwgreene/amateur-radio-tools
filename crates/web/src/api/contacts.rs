//! REST API for logging contacts from external applications.
//!
//! Authentication is via a personal API token: `Authorization: Bearer art_...`.
//! All endpoints are scoped to the token's owning user.

use actix_web::{HttpResponse, get, post, web};
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::{Deserialize, Serialize};

use crate::api::error::ApiError;
use crate::auth::api_token::ApiUser;
use crate::state::AppState;
use crate::tools::callsign;
use entity::contacts;

/// Request body for logging a contact. Only `callsign` is required.
#[derive(Debug, Deserialize)]
pub struct NewContact {
    pub callsign: String,
    /// When the contact happened (RFC 3339). Defaults to "now" if omitted.
    pub worked_at: Option<DateTime<Utc>>,
    pub band: Option<String>,
    pub mode: Option<String>,
    pub frequency_mhz: Option<f64>,
    pub rst_sent: Option<String>,
    pub rst_received: Option<String>,
    pub grid: Option<String>,
    pub name: Option<String>,
    pub qth: Option<String>,
    pub notes: Option<String>,
}

/// `POST /api/v1/contacts` — log a contact.
#[post("/api/v1/contacts")]
pub async fn create_contact(
    user: ApiUser,
    state: web::Data<AppState>,
    payload: web::Json<NewContact>,
) -> Result<HttpResponse, ApiError> {
    let input = payload.into_inner();

    let callsign = callsign::normalize(&input.callsign);
    if !callsign::is_valid(&callsign) {
        return Err(ApiError::BadRequest(format!(
            "'{}' is not a valid callsign",
            input.callsign
        )));
    }
    // Resolve the DXCC entity at logging time (best-effort).
    let country = callsign::lookup(&callsign).ok().map(|info| info.country);

    let now = Utc::now();
    let model = contacts::ActiveModel {
        user_id: Set(user.user.id),
        callsign: Set(callsign),
        worked_at: Set(input.worked_at.unwrap_or(now)),
        band: Set(input.band),
        mode: Set(input.mode),
        frequency_mhz: Set(input.frequency_mhz),
        rst_sent: Set(input.rst_sent),
        rst_received: Set(input.rst_received),
        grid: Set(input.grid),
        name: Set(input.name),
        qth: Set(input.qth),
        country: Set(country),
        notes: Set(input.notes),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&state.db)
    .await?;

    Ok(HttpResponse::Created().json(model))
}

/// `GET /api/v1/contacts` — list the caller's contacts, newest first.
#[get("/api/v1/contacts")]
pub async fn list_contacts(
    user: ApiUser,
    state: web::Data<AppState>,
) -> Result<HttpResponse, ApiError> {
    let items = contacts::Entity::find()
        .filter(contacts::Column::UserId.eq(user.user.id))
        .order_by_desc(contacts::Column::WorkedAt)
        .all(&state.db)
        .await?;

    Ok(HttpResponse::Ok().json(items))
}

/// `GET /api/v1/contacts/{id}` — fetch a single contact owned by the caller.
#[get("/api/v1/contacts/{id}")]
pub async fn get_contact(
    user: ApiUser,
    state: web::Data<AppState>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let id = path.into_inner();
    let item = contacts::Entity::find_by_id(id)
        .filter(contacts::Column::UserId.eq(user.user.id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;

    Ok(HttpResponse::Ok().json(item))
}

/// `GET /api/v1/me` — identify the token's owner (handy for verifying a token works).
#[get("/api/v1/me")]
pub async fn me(user: ApiUser) -> Result<HttpResponse, ApiError> {
    #[derive(Serialize)]
    struct Me {
        id: i64,
        email: String,
        name: Option<String>,
    }
    Ok(HttpResponse::Ok().json(Me {
        id: user.user.id,
        email: user.user.email,
        name: user.user.name,
    }))
}
