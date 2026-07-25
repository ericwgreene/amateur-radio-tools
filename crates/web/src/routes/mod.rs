//! HTTP route handlers and their Askama templates.

pub mod logbook;
pub mod pages;
pub mod partials;
pub mod sessions;
pub mod settings;
pub mod stations;

use actix_web::cookie::{Cookie, SameSite, time::Duration as CookieDuration};
use actix_web::web::ServiceConfig;
use actix_web::{HttpRequest, HttpResponse, get, web};

use crate::error::AppError;

/// Render an optional value for display, turning nothing into an em dash.
///
/// Every table on the site pre-formats its rows into a view struct rather than
/// putting logic in the template, and they all need this — so it lives here
/// rather than being copied into each route module.
pub fn dash(value: Option<String>) -> String {
    match value {
        Some(v) if !v.trim().is_empty() => v,
        _ => "—".to_string(),
    }
}

/// Remembers whether the monitoring pages refresh themselves. Named to match the
/// `art_` prefix the session cookie already uses.
pub const AUTO_REFRESH_COOKIE: &str = "art_autorefresh";

/// How often a live page re-fetches its rows. Tracks the desktop uploader's batch
/// window (~5 s) closely enough to feel current without being chatty.
pub const AUTO_REFRESH_SECS: u32 = 30;

/// A year — this is a display preference, not a session.
const AUTO_REFRESH_COOKIE_DAYS: i64 = 365;

/// Whether the monitoring pages should poll.
///
/// An explicit `?auto=` wins (that's the toggle being clicked), then the stored
/// preference, and failing both it is **on** — the pages exist to show data
/// arriving live, so that is the useful default.
pub fn auto_refresh_pref(req: &HttpRequest, param: Option<&str>) -> bool {
    if let Some(value) = param {
        return is_on(value);
    }
    match req.cookie(AUTO_REFRESH_COOKIE) {
        Some(cookie) => is_on(cookie.value()),
        None => true,
    }
}

fn is_on(value: &str) -> bool {
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "off" | "false" | "0" | "no"
    )
}

/// The cookie persisting that choice. `secure` comes from `COOKIE_SECURE`, the
/// same switch the session cookie honours, so this doesn't become the one cookie
/// that leaks over plain HTTP in production.
pub fn auto_refresh_cookie(on: bool, secure: bool) -> Cookie<'static> {
    Cookie::build(AUTO_REFRESH_COOKIE, if on { "on" } else { "off" })
        .path("/")
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .max_age(CookieDuration::days(AUTO_REFRESH_COOKIE_DAYS))
        .finish()
}

/// Liveness probe.
#[get("/health")]
async fn health() -> HttpResponse {
    HttpResponse::Ok().content_type("text/plain").body("ok")
}

/// Fallback handler for unmatched routes; renders the 404 error page.
async fn not_found() -> Result<HttpResponse, AppError> {
    Err(AppError::NotFound)
}

/// Register every route on the application. Called from `main` inside the app factory.
pub fn configure(cfg: &mut ServiceConfig) {
    cfg
        // Pages
        .service(health)
        .service(pages::index)
        .service(pages::dashboard)
        .service(pages::admin)
        .service(logbook::logbook_page)
        .service(logbook::add_contact)
        .service(logbook::delete_contact)
        // Monitoring. `/stations/rows` must be registered before
        // `/stations/{callsign}` or the path parameter would swallow it.
        .service(stations::stations_rows)
        .service(stations::stations_page)
        .service(stations::station_detail)
        .service(stations::promote)
        .service(sessions::sessions_page)
        .service(sessions::session_rows)
        .service(sessions::session_detail)
        .service(settings::tokens_page)
        .service(settings::create_token)
        .service(settings::revoke_token)
        // HTMX partials
        .service(partials::grid_tool)
        .service(partials::callsign_tool)
        // Auth (browser / OIDC)
        .service(crate::auth::handlers::login)
        .service(crate::auth::handlers::callback)
        .service(crate::auth::handlers::logout);

    // REST API (token-authenticated, JSON).
    crate::api::configure(cfg);

    // Anything else → 404 page.
    cfg.default_service(web::route().to(not_found));
}

#[cfg(test)]
mod tests {
    use crate::test_support::{test_app, test_state};
    use actix_web::test;

    #[actix_web::test]
    async fn health_returns_ok() {
        let state = test_state().await;
        let app = test_app!(state);
        let req = test::TestRequest::get().uri("/health").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        assert_eq!(test::read_body(resp).await.as_ref(), b"ok");
    }

    #[actix_web::test]
    async fn unknown_route_renders_404() {
        let state = test_state().await;
        let app = test_app!(state);
        let req = test::TestRequest::get().uri("/does-not-exist").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);
    }
}
