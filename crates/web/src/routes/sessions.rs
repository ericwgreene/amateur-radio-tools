//! Operating sessions: the runs that monitored transmissions are grouped into.
//!
//! Where the station roster answers "who have I ever heard", this answers "what
//! happened during Tuesday's net" — a run at a time, with the stations it turned
//! up.

use actix_web::{get, web};
use askama::Template;
use askama_web::WebTemplate;
use sea_orm::sea_query::{Expr, Func, SimpleExpr};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::auth::session::{AuthedUser, SessionUser};
use crate::error::AppError;
use crate::routes::dash;
use crate::routes::stations::session_title;
use crate::state::AppState;
use entity::{observations, sessions, stations};

const PAGE_LIMIT: u64 = 200;

/// A session row, pre-formatted.
pub struct SessionView {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub started: String,
    /// "in progress" while the run is still open — an unexpectedly-open session
    /// is real information (the app was killed), so it isn't auto-closed.
    pub duration: String,
    pub band: String,
    pub mode: String,
    pub frequency: String,
    pub operator: String,
    pub heard: u64,
    pub unique_stations: u64,
}

/// How long a session ran, or how long it has been running.
fn duration_text(s: &sessions::Model) -> String {
    let Some(ended) = s.ended_at else {
        return "in progress".to_string();
    };
    let minutes = (ended - s.started_at).num_minutes().max(0);
    if minutes < 60 {
        format!("{minutes}m")
    } else {
        format!("{}h {}m", minutes / 60, minutes % 60)
    }
}

async fn load_sessions(state: &AppState, user_id: i64) -> Result<Vec<SessionView>, AppError> {
    let rows = sessions::Entity::find()
        .filter(sessions::Column::UserId.eq(user_id))
        .order_by_desc(sessions::Column::StartedAt)
        .limit(PAGE_LIMIT)
        .all(&state.db)
        .await?;

    // Totals for the whole page in one grouped query. Counting per session would
    // mean a query per row, which is fine at three sessions and not at two
    // hundred — and it would pull every observation body across just to count it.
    let session_ids: Vec<i64> = rows.iter().map(|s| s.id).collect();
    let totals = observation_totals(state, user_id, &session_ids).await?;

    Ok(rows
        .into_iter()
        .map(|s| {
            let (heard, unique_stations) = totals.get(&s.id).copied().unwrap_or((0, 0));
            SessionView {
                id: s.id,
                title: session_title(&s),
                kind: s.kind.clone(),
                started: s.started_at.format("%Y-%m-%d %H:%M").to_string(),
                duration: duration_text(&s),
                band: dash(s.band.clone()),
                mode: dash(s.mode.clone()),
                frequency: s
                    .frequency_mhz
                    .map(|f| format!("{f:.3}"))
                    .unwrap_or_else(|| "—".to_string()),
                operator: dash(s.operator_callsign.clone()),
                heard,
                unique_stations,
            }
        })
        .collect())
}

/// `(total hearings, distinct callsigns)` per session, for a page of sessions.
async fn observation_totals(
    state: &AppState,
    user_id: i64,
    session_ids: &[i64],
) -> Result<std::collections::HashMap<i64, (u64, u64)>, AppError> {
    if session_ids.is_empty() {
        return Ok(Default::default());
    }
    let rows: Vec<(i64, i64, i64)> = observations::Entity::find()
        .select_only()
        .column(observations::Column::SessionId)
        .column_as(observations::Column::Id.count(), "heard")
        .column_as(
            SimpleExpr::FunctionCall(Func::count_distinct(Expr::col(
                observations::Column::Callsign,
            ))),
            "unique_stations",
        )
        .filter(observations::Column::UserId.eq(user_id))
        .filter(observations::Column::SessionId.is_in(session_ids.to_vec()))
        .group_by(observations::Column::SessionId)
        .into_tuple()
        .all(&state.db)
        .await?;

    Ok(rows
        .into_iter()
        .map(|(id, heard, unique)| (id, (heard.max(0) as u64, unique.max(0) as u64)))
        .collect())
}

#[derive(Template, WebTemplate)]
#[template(path = "sessions.html")]
struct SessionsPage {
    current_user: Option<SessionUser>,
    sessions: Vec<SessionView>,
}

#[get("/sessions")]
pub async fn sessions_page(
    user: AuthedUser,
    state: web::Data<AppState>,
) -> Result<SessionsPage, AppError> {
    let sessions = load_sessions(&state, user.0.id).await?;
    Ok(SessionsPage {
        current_user: Some(user.0),
        sessions,
    })
}

/// One row of a session's log: which station, when, and whether it's been worked.
pub struct SessionHearingView {
    pub callsign: String,
    pub heard_at: String,
    pub duration: String,
    pub name: String,
    pub country: String,
    pub transcript: String,
    pub promoted: bool,
}

#[derive(Template, WebTemplate)]
#[template(path = "session_detail.html")]
struct SessionDetailPage {
    current_user: Option<SessionUser>,
    session: SessionView,
    notes: String,
    hearings: Vec<SessionHearingView>,
}

#[get("/sessions/{id}")]
pub async fn session_detail(
    user: AuthedUser,
    state: web::Data<AppState>,
    path: web::Path<i64>,
) -> Result<SessionDetailPage, AppError> {
    let id = path.into_inner();
    let model = sessions::Entity::find_by_id(id)
        .filter(sessions::Column::UserId.eq(user.0.id))
        .one(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;

    let rows = observations::Entity::find()
        .filter(observations::Column::UserId.eq(user.0.id))
        .filter(observations::Column::SessionId.eq(id))
        .order_by_desc(observations::Column::HeardAt)
        .limit(PAGE_LIMIT)
        .all(&state.db)
        .await?;

    let unique: std::collections::HashSet<&str> =
        rows.iter().map(|o| o.callsign.as_str()).collect();

    let view = SessionView {
        id: model.id,
        title: session_title(&model),
        kind: model.kind.clone(),
        started: model.started_at.format("%Y-%m-%d %H:%M").to_string(),
        duration: duration_text(&model),
        band: dash(model.band.clone()),
        mode: dash(model.mode.clone()),
        frequency: model
            .frequency_mhz
            .map(|f| format!("{f:.3}"))
            .unwrap_or_else(|| "—".to_string()),
        operator: dash(model.operator_callsign.clone()),
        heard: rows.len() as u64,
        unique_stations: unique.len() as u64,
    };

    // Licensee names live once on the station rollup rather than being copied
    // onto every hearing, so fetch them for the callsigns in this session — one
    // query, not one per row.
    let calls: Vec<String> = unique.iter().map(|c| c.to_string()).collect();
    let names: std::collections::HashMap<String, Option<String>> = if calls.is_empty() {
        Default::default()
    } else {
        stations::Entity::find()
            .filter(stations::Column::UserId.eq(user.0.id))
            .filter(stations::Column::Callsign.is_in(calls))
            .all(&state.db)
            .await?
            .into_iter()
            .map(|s| (s.callsign, s.name))
            .collect()
    };

    let hearings = rows
        .into_iter()
        .map(|o| SessionHearingView {
            heard_at: o.heard_at.format("%H:%M:%S").to_string(),
            duration: o
                .duration_secs
                .map(|d| format!("{d:.1}s"))
                .unwrap_or_else(|| "—".to_string()),
            name: dash(names.get(&o.callsign).cloned().flatten()),
            country: dash(o.country),
            transcript: dash(o.transcript),
            promoted: o.promoted_contact_id.is_some(),
            callsign: o.callsign,
        })
        .collect();

    Ok(SessionDetailPage {
        current_user: Some(user.0),
        notes: dash(model.notes),
        session: view,
        hearings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};

    fn model(ended_after: Option<i64>) -> sessions::Model {
        let started = Utc.with_ymd_and_hms(2026, 7, 24, 20, 0, 0).unwrap();
        sessions::Model {
            id: 1,
            user_id: 1,
            client_key: "k".into(),
            kind: "net".into(),
            label: None,
            started_at: started,
            ended_at: ended_after.map(|m| started + Duration::minutes(m)),
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
    fn an_open_session_reads_as_in_progress() {
        assert_eq!(duration_text(&model(None)), "in progress");
    }

    #[test]
    fn duration_switches_to_hours_past_an_hour() {
        assert_eq!(duration_text(&model(Some(45))), "45m");
        assert_eq!(duration_text(&model(Some(60))), "1h 0m");
        assert_eq!(duration_text(&model(Some(135))), "2h 15m");
    }

    /// A close message that arrived out of order could carry an end before the
    /// start; show zero rather than a negative duration.
    #[test]
    fn a_backwards_end_time_does_not_render_as_negative() {
        assert_eq!(duration_text(&model(Some(-30))), "0m");
    }
}
