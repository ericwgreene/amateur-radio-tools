//! The JSON REST API (versioned under `/api/v1`).
//!
//! Authenticated with personal API tokens (see [`crate::auth::api_token`]) so external
//! applications — logging software, scripts, station computers — can record contacts
//! without the browser OIDC flow.

pub mod contacts;
pub mod error;

use actix_web::web::ServiceConfig;

/// Register all API routes.
pub fn configure(cfg: &mut ServiceConfig) {
    cfg.service(contacts::create_contact)
        .service(contacts::list_contacts)
        .service(contacts::get_contact)
        .service(contacts::me);
}
