//! Full-page handlers (each returns a complete HTML document via a layout template).

use actix_web::{get, web};
use askama::Template;
use askama_web::WebTemplate;
use sea_orm::EntityTrait;

use crate::auth::session::{AuthedUser, MaybeUser, SessionUser};
use crate::error::AppError;
use crate::state::AppState;
use entity::users;

/// Public home page. Renders differently depending on whether the visitor is signed in.
#[derive(Template, WebTemplate)]
#[template(path = "index.html")]
struct IndexTemplate {
    current_user: Option<SessionUser>,
}

#[get("/")]
pub async fn index(user: MaybeUser) -> IndexTemplate {
    IndexTemplate {
        current_user: user.0,
    }
}

/// Authenticated dashboard: shows the signed-in user's profile and account timestamps.
#[derive(Template, WebTemplate)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    current_user: Option<SessionUser>,
    created_at: String,
    last_login: String,
}

#[get("/dashboard")]
pub async fn dashboard(
    user: AuthedUser,
    state: web::Data<AppState>,
) -> Result<DashboardTemplate, AppError> {
    // Read the local user record to surface account timestamps (demonstrates a DB read).
    let record = users::Entity::find_by_id(user.0.id).one(&state.db).await?;
    let (created_at, last_login) = match record {
        Some(u) => (fmt_dt(Some(u.created_at)), fmt_dt(u.last_login_at)),
        None => ("—".to_string(), "—".to_string()),
    };

    Ok(DashboardTemplate {
        current_user: Some(user.0),
        created_at,
        last_login,
    })
}

/// Admin-only page listing all users — demonstrates role-based authorization.
#[derive(Template, WebTemplate)]
#[template(path = "admin.html")]
struct AdminTemplate {
    current_user: Option<SessionUser>,
    users: Vec<AdminUserRow>,
}

struct AdminUserRow {
    id: i64,
    email: String,
    name: String,
    sub: String,
    last_login: String,
}

#[get("/admin")]
pub async fn admin(
    user: AuthedUser,
    state: web::Data<AppState>,
) -> Result<AdminTemplate, AppError> {
    // Authorization gate: only users carrying the `admin` role may proceed.
    if !user.0.is_admin() {
        return Err(AppError::Forbidden);
    }

    let rows = users::Entity::find()
        .all(&state.db)
        .await?
        .into_iter()
        .map(|u| AdminUserRow {
            id: u.id,
            email: u.email,
            name: u.name.unwrap_or_default(),
            sub: u.auth0_sub,
            last_login: fmt_dt(u.last_login_at),
        })
        .collect();

    Ok(AdminTemplate {
        current_user: Some(user.0),
        users: rows,
    })
}

fn fmt_dt(dt: Option<chrono::DateTime<chrono::Utc>>) -> String {
    match dt {
        Some(d) => d.format("%Y-%m-%d %H:%M UTC").to_string(),
        None => "—".to_string(),
    }
}
