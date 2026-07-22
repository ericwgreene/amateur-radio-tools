//! The logbook: browse, add (with live callsign lookup), and delete contacts.
//!
//! The list is server-rendered; adding and deleting use HTMX to swap just the table body
//! back in, so the page never fully reloads.

use actix_web::{delete, get, post, web};
use askama::Template;
use askama_web::WebTemplate;
use chrono::{DateTime, NaiveDateTime, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::Deserialize;

use crate::auth::session::{AuthedUser, SessionUser};
use crate::error::AppError;
use crate::state::AppState;
use crate::tools::callsign;
use entity::contacts;

/// A logbook entry shaped for display (everything pre-formatted; blanks become "—").
struct ContactView {
    id: i64,
    worked_at: String,
    callsign: String,
    country: String,
    band: String,
    mode: String,
    frequency: String,
    rst: String,
    grid: String,
    name: String,
}

fn dash(value: Option<String>) -> String {
    match value {
        Some(v) if !v.trim().is_empty() => v,
        _ => "—".to_string(),
    }
}

fn to_view(m: contacts::Model) -> ContactView {
    let rst = match (m.rst_sent.as_deref(), m.rst_received.as_deref()) {
        (None, None) => "—".to_string(),
        (s, r) => format!("{} / {}", s.unwrap_or("·"), r.unwrap_or("·")),
    };
    ContactView {
        id: m.id,
        worked_at: m.worked_at.format("%Y-%m-%d %H:%M").to_string(),
        callsign: m.callsign,
        country: dash(m.country),
        band: dash(m.band),
        mode: dash(m.mode),
        frequency: m
            .frequency_mhz
            .map(|f| format!("{f:.3}"))
            .unwrap_or_else(|| "—".to_string()),
        rst,
        grid: dash(m.grid),
        name: dash(m.name),
    }
}

async fn load_contacts(state: &AppState, user_id: i64) -> Result<Vec<ContactView>, AppError> {
    let rows = contacts::Entity::find()
        .filter(contacts::Column::UserId.eq(user_id))
        .order_by_desc(contacts::Column::WorkedAt)
        .all(&state.db)
        .await?;
    Ok(rows.into_iter().map(to_view).collect())
}

#[derive(Template, WebTemplate)]
#[template(path = "logbook.html")]
struct LogbookPage {
    current_user: Option<SessionUser>,
    contacts: Vec<ContactView>,
}

/// Fragment: just the table body rows (used to refresh the list after add/delete).
#[derive(Template, WebTemplate)]
#[template(path = "partials/contact_rows.html")]
struct ContactRows {
    contacts: Vec<ContactView>,
}

#[get("/logbook")]
pub async fn logbook_page(
    user: AuthedUser,
    state: web::Data<AppState>,
) -> Result<LogbookPage, AppError> {
    let contacts = load_contacts(&state, user.0.id).await?;
    Ok(LogbookPage {
        current_user: Some(user.0),
        contacts,
    })
}

#[derive(Debug, Deserialize)]
pub struct ContactForm {
    callsign: String,
    worked_at: Option<String>,
    band: Option<String>,
    mode: Option<String>,
    frequency_mhz: Option<String>,
    rst_sent: Option<String>,
    rst_received: Option<String>,
    grid: Option<String>,
    name: Option<String>,
    qth: Option<String>,
    notes: Option<String>,
}

/// Trim, and treat an empty string as absent.
fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[post("/logbook")]
pub async fn add_contact(
    user: AuthedUser,
    state: web::Data<AppState>,
    form: web::Form<ContactForm>,
) -> Result<ContactRows, AppError> {
    let form = form.into_inner();
    let callsign_norm = callsign::normalize(&form.callsign);

    if !callsign_norm.is_empty() {
        let country = callsign::lookup(&callsign_norm).ok().map(|i| i.country);
        let worked_at = clean(form.worked_at)
            .and_then(|s| parse_local_datetime(&s))
            .unwrap_or_else(Utc::now);
        let frequency = clean(form.frequency_mhz).and_then(|s| s.parse::<f64>().ok());
        let now = Utc::now();

        contacts::ActiveModel {
            user_id: Set(user.0.id),
            callsign: Set(callsign_norm),
            worked_at: Set(worked_at),
            band: Set(clean(form.band)),
            mode: Set(clean(form.mode)),
            frequency_mhz: Set(frequency),
            rst_sent: Set(clean(form.rst_sent)),
            rst_received: Set(clean(form.rst_received)),
            grid: Set(clean(form.grid)),
            name: Set(clean(form.name)),
            qth: Set(clean(form.qth)),
            country: Set(country),
            notes: Set(clean(form.notes)),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&state.db)
        .await?;
    }

    let contacts = load_contacts(&state, user.0.id).await?;
    Ok(ContactRows { contacts })
}

#[delete("/logbook/{id}")]
pub async fn delete_contact(
    user: AuthedUser,
    state: web::Data<AppState>,
    path: web::Path<i64>,
) -> Result<ContactRows, AppError> {
    let id = path.into_inner();
    // Scope the delete to the owner so users can only remove their own contacts.
    contacts::Entity::delete_many()
        .filter(contacts::Column::Id.eq(id))
        .filter(contacts::Column::UserId.eq(user.0.id))
        .exec(&state.db)
        .await?;

    let contacts = load_contacts(&state, user.0.id).await?;
    Ok(ContactRows { contacts })
}

/// Parse an HTML `datetime-local` value (`YYYY-MM-DDTHH:MM`), treating it as UTC.
fn parse_local_datetime(value: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M")
        .ok()
        .map(|naive| naive.and_utc())
}

/// A tiny unit test on the datetime parsing, which is easy to get subtly wrong.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_datetime_local() {
        let dt = parse_local_datetime("2026-07-22T14:30").unwrap();
        assert_eq!(dt.to_rfc3339(), "2026-07-22T14:30:00+00:00");
        assert!(parse_local_datetime("nonsense").is_none());
    }
}
