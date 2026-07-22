//! HTTP route handlers and their Askama templates.

pub mod logbook;
pub mod pages;
pub mod partials;
pub mod settings;

use actix_web::web::ServiceConfig;
use actix_web::{HttpResponse, get, web};

use crate::error::AppError;

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
