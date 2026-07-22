//! Personal API tokens: generation, hashing, and the `ApiUser` request extractor.
//!
//! Tokens let an external application authenticate as a user for the REST API. The raw
//! token is only ever seen once (at creation); we persist just its SHA-256 hash. Because
//! tokens are 256 bits of randomness, a plain cryptographic hash is sufficient — unlike a
//! password, there is nothing to brute-force.

use std::future::Future;
use std::pin::Pin;

use actix_web::{FromRequest, HttpRequest, dev::Payload, http::header, web};
use base64::Engine;
use chrono::Utc;
use rand::RngCore;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use sha2::{Digest, Sha256};

use crate::api::error::ApiError;
use crate::state::AppState;
use entity::{api_tokens, users};

/// Number of random bytes in a token body.
const TOKEN_BYTES: usize = 32;
/// Human-readable prefix shown in the UI (e.g. `art_a1b2c3xy`).
pub const TOKEN_DISPLAY_PREFIX_LEN: usize = 12;

pub struct GeneratedToken {
    /// The full token, shown to the user exactly once.
    pub raw: String,
    /// SHA-256 hex of `raw`, stored in the database.
    pub hash: String,
    /// A non-secret leading fragment of `raw`, stored for display.
    pub prefix: String,
}

/// Generate a fresh random token.
pub fn generate_token() -> GeneratedToken {
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::rng().fill_bytes(&mut bytes);
    let body = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let raw = format!("art_{body}");
    let hash = hash_token(&raw);
    let prefix = raw.chars().take(TOKEN_DISPLAY_PREFIX_LEN).collect();
    GeneratedToken { raw, hash, prefix }
}

/// SHA-256 hex digest of a raw token.
pub fn hash_token(raw: &str) -> String {
    let digest = Sha256::digest(raw.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// A user authenticated via an API token (as opposed to a browser session).
pub struct ApiUser {
    pub user: users::Model,
}

impl FromRequest for ApiUser {
    type Error = ApiError;
    type Future = Pin<Box<dyn Future<Output = Result<Self, Self::Error>>>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        let presented = token_from_request(req);
        let state = req.app_data::<web::Data<AppState>>().cloned();

        Box::pin(async move {
            let state = state.ok_or_else(|| ApiError::internal("application state missing"))?;
            let token = presented.ok_or(ApiError::Unauthorized)?;
            let hash = hash_token(&token);

            // Look up a non-revoked token by hash.
            let record = api_tokens::Entity::find()
                .filter(api_tokens::Column::TokenHash.eq(&hash))
                .filter(api_tokens::Column::RevokedAt.is_null())
                .one(&state.db)
                .await?
                .ok_or(ApiError::Unauthorized)?;

            let user = users::Entity::find_by_id(record.user_id)
                .one(&state.db)
                .await?
                .ok_or(ApiError::Unauthorized)?;

            // Best-effort last-used bookkeeping (ignore failures).
            let mut active = record.into_active_model();
            active.last_used_at = Set(Some(Utc::now()));
            let _ = active.update(&state.db).await;

            Ok(ApiUser { user })
        })
    }
}

/// Extract the API token from a request.
///
/// The `Authorization: Bearer <token>` header is preferred. As a fallback — for clients
/// that can't set custom headers — the token may also be supplied as the `api_key`
/// query-string parameter (e.g. `?api_key=art_...`). If both are present, the header wins.
///
/// Credentials in the URL are more exposed than in a header (they appear in server/proxy
/// access logs and browser history), so the header remains the recommended path.
fn token_from_request(req: &HttpRequest) -> Option<String> {
    // 1. Authorization header (preferred).
    if let Some(value) = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(token) = value
            .strip_prefix("Bearer ")
            .or_else(|| value.strip_prefix("bearer "))
        {
            let token = token.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }

    // 2. `api_key` query-string parameter (fallback).
    url::form_urlencoded::parse(req.query_string().as_bytes())
        .find(|(key, _)| key == "api_key")
        .map(|(_, value)| value.trim().to_string())
        .filter(|token| !token.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::test::TestRequest;

    #[test]
    fn reads_token_from_authorization_header() {
        let req = TestRequest::default()
            .insert_header(("Authorization", "Bearer art_headertoken"))
            .to_http_request();
        assert_eq!(token_from_request(&req).as_deref(), Some("art_headertoken"));
    }

    #[test]
    fn reads_token_from_api_key_query_param() {
        let req = TestRequest::with_uri("/api/v1/me?api_key=art_querytoken").to_http_request();
        assert_eq!(token_from_request(&req).as_deref(), Some("art_querytoken"));
    }

    #[test]
    fn header_takes_precedence_over_query() {
        let req = TestRequest::with_uri("/api/v1/me?api_key=art_query")
            .insert_header(("Authorization", "Bearer art_header"))
            .to_http_request();
        assert_eq!(token_from_request(&req).as_deref(), Some("art_header"));
    }

    #[test]
    fn no_token_when_absent() {
        let req = TestRequest::default().to_http_request();
        assert_eq!(token_from_request(&req), None);
    }

    #[test]
    fn tokens_are_prefixed_and_hashed() {
        let t = generate_token();
        assert!(t.raw.starts_with("art_"));
        assert_eq!(t.prefix.len(), TOKEN_DISPLAY_PREFIX_LEN);
        assert!(t.raw.starts_with(&t.prefix));
        // The hash is deterministic and 64 hex chars (SHA-256).
        assert_eq!(t.hash, hash_token(&t.raw));
        assert_eq!(t.hash.len(), 64);
    }

    #[test]
    fn tokens_are_unique() {
        assert_ne!(generate_token().raw, generate_token().raw);
    }
}
