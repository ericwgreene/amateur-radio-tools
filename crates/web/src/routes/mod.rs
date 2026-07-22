//! HTTP route handlers and their Askama templates.

pub mod pages;
pub mod partials;

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
    cfg.service(health)
        // Pages
        .service(pages::index)
        .service(pages::dashboard)
        .service(pages::admin)
        // HTMX partials
        .service(partials::grid_tool)
        // Auth
        .service(crate::auth::handlers::login)
        .service(crate::auth::handlers::callback)
        .service(crate::auth::handlers::logout)
        // Anything else → 404 page.
        .default_service(web::route().to(not_found));
}
