//! The JSON REST API (versioned under `/api/v1`).
//!
//! Authenticated with personal API tokens (see [`crate::auth::api_token`]) so external
//! applications — logging software, scripts, station computers — can record contacts
//! without the browser OIDC flow.

pub mod contacts;
pub mod error;
pub mod monitoring;

use actix_web::web::ServiceConfig;
use serde::{Deserialize, Serialize};

/// Register all API routes.
pub fn configure(cfg: &mut ServiceConfig) {
    cfg.service(contacts::create_contact)
        .service(contacts::list_contacts)
        .service(contacts::get_contact)
        .service(contacts::me)
        // Monitoring: sessions, observations, and the unique-station roster.
        .service(monitoring::create_session)
        .service(monitoring::update_session)
        .service(monitoring::list_sessions)
        .service(monitoring::get_session)
        .service(monitoring::ingest_observations)
        .service(monitoring::list_observations)
        .service(monitoring::promote_observation)
        .service(monitoring::list_stations)
        .service(monitoring::get_station);
}

/// Query-string pagination, shared by the monitoring endpoints.
///
/// A monitoring log grows far faster than a logbook — a single net can add hundreds of
/// rows — so these endpoints page by default rather than returning everything. The
/// older `/api/v1/contacts` endpoints keep returning a bare array; changing their shape
/// would break any existing client for no benefit here.
#[derive(Debug, Default, Deserialize)]
pub struct Page {
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

/// Default page size when the client doesn't ask.
pub const DEFAULT_LIMIT: u64 = 100;
/// Ceiling, so one request can't ask for the whole table.
pub const MAX_LIMIT: u64 = 500;

impl Page {
    /// Resolve to `(limit, offset)`, clamping rather than erroring — a client asking
    /// for more than we'll give should get a full page, not a 400.
    pub fn resolve(&self) -> (u64, u64) {
        let limit = self.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        (limit, self.offset.unwrap_or(0))
    }
}

/// A page of results plus enough context to fetch the next one.
#[derive(Debug, Serialize)]
pub struct Paged<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub limit: u64,
    pub offset: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_defaults_when_unspecified() {
        let (limit, offset) = Page::default().resolve();
        assert_eq!(limit, DEFAULT_LIMIT);
        assert_eq!(offset, 0);
    }

    #[test]
    fn page_clamps_rather_than_rejecting() {
        let (limit, _) = Page {
            limit: Some(100_000),
            offset: None,
        }
        .resolve();
        assert_eq!(limit, MAX_LIMIT);

        let (limit, _) = Page {
            limit: Some(0),
            offset: None,
        }
        .resolve();
        assert_eq!(limit, 1, "a zero-size page would never make progress");
    }

    #[test]
    fn page_passes_offset_through() {
        let (_, offset) = Page {
            limit: Some(10),
            offset: Some(40),
        }
        .resolve();
        assert_eq!(offset, 40);
    }
}
