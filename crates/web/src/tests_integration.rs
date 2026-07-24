//! End-to-end integration tests plus focused unit tests for otherwise-untested modules.
//!
//! HTTP tests drive the real route table (`routes::configure`) through `actix_web::test`
//! against an in-memory SQLite database, exercising the request → extractor → handler → DB
//! → template path offline. Direct tests cover pure/public items (config parsing, error
//! mapping, session helpers).

use actix_web::http::header;
use actix_web::{ResponseError, test};

use crate::api::error::ApiError;
use crate::auth::session::SessionUser;
use crate::config::Config;
use crate::error::AppError;
use crate::test_support::{
    login, seed_contact, seed_token, seed_user, session_user, test_app, test_state,
};

/// Build a urlencoded form POST request builder (call `.to_request()` at the use site).
fn form(uri: &str, body: &'static str) -> test::TestRequest {
    test::TestRequest::post()
        .uri(uri)
        .insert_header((header::CONTENT_TYPE, "application/x-www-form-urlencoded"))
        .set_payload(body)
}

// ---------------------------------------------------------------------------
// Public HTMX tools (no auth)
// ---------------------------------------------------------------------------

#[actix_web::test]
async fn grid_tool_ok_and_error() {
    let state = test_state().await;
    let app = test_app!(state);

    let resp = test::call_service(
        &app,
        form("/tools/grid", "lat=41.714775&lon=-72.727260").to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body = test::read_body(resp).await;
    assert!(String::from_utf8_lossy(&body).contains("FN31pr"));

    let resp = test::call_service(&app, form("/tools/grid", "lat=abc&lon=0").to_request()).await;
    let body = test::read_body(resp).await;
    assert!(String::from_utf8_lossy(&body).contains("Latitude must be a number"));
}

#[actix_web::test]
async fn callsign_tool_ok_and_error() {
    let state = test_state().await;
    let app = test_app!(state);

    let resp =
        test::call_service(&app, form("/tools/callsign", "callsign=w1aw").to_request()).await;
    assert_eq!(resp.status(), 200);
    assert!(String::from_utf8_lossy(&test::read_body(resp).await).contains("United States"));

    let resp = test::call_service(
        &app,
        form("/tools/callsign", "callsign=%21%21").to_request(),
    )
    .await;
    assert!(String::from_utf8_lossy(&test::read_body(resp).await).contains("not a valid callsign"));
}

// ---------------------------------------------------------------------------
// Pages + session auth
// ---------------------------------------------------------------------------

#[actix_web::test]
async fn home_renders_logged_out_and_in() {
    let state = test_state().await;
    let user = seed_user(&state.db, "home@example.com").await;
    let app = test_app!(state);

    let resp = test::call_service(&app, test::TestRequest::get().uri("/").to_request()).await;
    assert_eq!(resp.status(), 200);

    let cookie = login!(app, user.id, false);
    let req = test::TestRequest::get()
        .uri("/")
        .cookie(cookie)
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 200);
}

#[actix_web::test]
async fn dashboard_requires_auth() {
    let state = test_state().await;
    let app = test_app!(state);

    // No session → 303 redirect to /login.
    let resp = test::call_service(
        &app,
        test::TestRequest::get().uri("/dashboard").to_request(),
    )
    .await;
    assert_eq!(resp.status(), 303);

    // HTMX request → 200 with HX-Redirect header instead.
    let req = test::TestRequest::get()
        .uri("/dashboard")
        .insert_header(("HX-Request", "true"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    assert!(resp.headers().contains_key("HX-Redirect"));
}

#[actix_web::test]
async fn dashboard_ok_when_authed() {
    let state = test_state().await;
    let user = seed_user(&state.db, "dash@example.com").await;
    let app = test_app!(state);
    let cookie = login!(app, user.id, false);
    let req = test::TestRequest::get()
        .uri("/dashboard")
        .cookie(cookie)
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 200);
}

#[actix_web::test]
async fn admin_is_role_gated() {
    let state = test_state().await;
    let user = seed_user(&state.db, "admin@example.com").await;
    let app = test_app!(state);

    let cookie = login!(app, user.id, false);
    let req = test::TestRequest::get()
        .uri("/admin")
        .cookie(cookie)
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 403);

    let cookie = login!(app, user.id, true);
    let req = test::TestRequest::get()
        .uri("/admin")
        .cookie(cookie)
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 200);
}

// ---------------------------------------------------------------------------
// Logbook
// ---------------------------------------------------------------------------

#[actix_web::test]
async fn logbook_add_list_delete() {
    let state = test_state().await;
    let user = seed_user(&state.db, "log@example.com").await;
    let app = test_app!(state);
    let cookie = login!(app, user.id, false);

    // Empty list renders.
    let req = test::TestRequest::get()
        .uri("/logbook")
        .cookie(cookie.clone())
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 200);

    // Add a contact (country auto-resolved from callsign).
    let add = test::TestRequest::post()
        .uri("/logbook")
        .cookie(cookie.clone())
        .insert_header((header::CONTENT_TYPE, "application/x-www-form-urlencoded"))
        .set_payload("callsign=g3abc&band=20m&rst_sent=59&rst_received=57")
        .to_request();
    let resp = test::call_service(&app, add).await;
    assert_eq!(resp.status(), 200);
    let body = test::read_body(resp).await;
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("G3ABC"));
    assert!(body.contains("England"));

    // Empty callsign is a no-op that still returns the table body.
    let noop = test::TestRequest::post()
        .uri("/logbook")
        .cookie(cookie.clone())
        .insert_header((header::CONTENT_TYPE, "application/x-www-form-urlencoded"))
        .set_payload("callsign=%20")
        .to_request();
    assert_eq!(test::call_service(&app, noop).await.status(), 200);

    // Delete a seeded contact owned by the user.
    let contact = seed_contact(&state.db, user.id, "K1ABC").await;
    let del = test::TestRequest::delete()
        .uri(&format!("/logbook/{}", contact.id))
        .cookie(cookie)
        .to_request();
    assert_eq!(test::call_service(&app, del).await.status(), 200);
}

// ---------------------------------------------------------------------------
// Settings / API tokens
// ---------------------------------------------------------------------------

#[actix_web::test]
async fn token_settings_create_and_revoke() {
    let state = test_state().await;
    let user = seed_user(&state.db, "tok@example.com").await;
    let app = test_app!(state);
    let cookie = login!(app, user.id, false);

    let req = test::TestRequest::get()
        .uri("/settings/tokens")
        .cookie(cookie.clone())
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 200);

    // Create shows the raw token once.
    let create = test::TestRequest::post()
        .uri("/settings/tokens")
        .cookie(cookie.clone())
        .insert_header((header::CONTENT_TYPE, "application/x-www-form-urlencoded"))
        .set_payload("name=WSJT-X")
        .to_request();
    let resp = test::call_service(&app, create).await;
    assert_eq!(resp.status(), 200);
    assert!(String::from_utf8_lossy(&test::read_body(resp).await).contains("art_"));

    // Revoke a seeded token owned by the user.
    let (_raw, token_id) = seed_token(&state.db, user.id).await;
    let revoke = test::TestRequest::post()
        .uri(&format!("/settings/tokens/{token_id}/revoke"))
        .cookie(cookie)
        .to_request();
    assert_eq!(test::call_service(&app, revoke).await.status(), 200);
}

// ---------------------------------------------------------------------------
// REST API (bearer token)
// ---------------------------------------------------------------------------

#[actix_web::test]
async fn api_rejects_missing_and_bad_tokens() {
    let state = test_state().await;
    let app = test_app!(state);

    let req = test::TestRequest::post()
        .uri("/api/v1/contacts")
        .set_json(serde_json::json!({"callsign": "W1AW"}))
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 401);

    let req = test::TestRequest::get()
        .uri("/api/v1/me")
        .insert_header(("Authorization", "Bearer art_bogus"))
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 401);
}

#[actix_web::test]
async fn api_contacts_crud_with_token() {
    let state = test_state().await;
    let user = seed_user(&state.db, "api@example.com").await;
    let (raw, _id) = seed_token(&state.db, user.id).await;
    let app = test_app!(state);
    let bearer = format!("Bearer {raw}");

    // Identify the token owner.
    let me = test::TestRequest::get()
        .uri("/api/v1/me")
        .insert_header(("Authorization", bearer.clone()))
        .to_request();
    let resp = test::call_service(&app, me).await;
    assert_eq!(resp.status(), 200);
    assert!(String::from_utf8_lossy(&test::read_body(resp).await).contains("api@example.com"));

    // Create (callsign normalized + country resolved).
    let create = test::TestRequest::post()
        .uri("/api/v1/contacts")
        .insert_header(("Authorization", bearer.clone()))
        .set_json(serde_json::json!({"callsign": "ve3xyz", "band": "40m"}))
        .to_request();
    let resp = test::call_service(&app, create).await;
    assert_eq!(resp.status(), 201);
    let created: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(created["callsign"], "VE3XYZ");
    assert_eq!(created["country"], "Canada");
    let id = created["id"].as_i64().unwrap();

    // Invalid callsign → 400.
    let bad = test::TestRequest::post()
        .uri("/api/v1/contacts")
        .insert_header(("Authorization", bearer.clone()))
        .set_json(serde_json::json!({"callsign": "!!"}))
        .to_request();
    assert_eq!(test::call_service(&app, bad).await.status(), 400);

    // List.
    let list = test::TestRequest::get()
        .uri("/api/v1/contacts")
        .insert_header(("Authorization", bearer.clone()))
        .to_request();
    assert_eq!(test::call_service(&app, list).await.status(), 200);

    // Get by id (found + not found).
    let get = test::TestRequest::get()
        .uri(&format!("/api/v1/contacts/{id}"))
        .insert_header(("Authorization", bearer.clone()))
        .to_request();
    assert_eq!(test::call_service(&app, get).await.status(), 200);

    let missing = test::TestRequest::get()
        .uri("/api/v1/contacts/999999")
        .insert_header(("Authorization", bearer))
        .to_request();
    assert_eq!(test::call_service(&app, missing).await.status(), 404);
}

// ---------------------------------------------------------------------------
// Config parsing (env-mutating → serialized behind a lock)
// ---------------------------------------------------------------------------

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
const ENV_KEYS: &[&str] = &[
    "BIND_ADDRESS",
    "BASE_URL",
    "DATABASE_URL",
    "STATIC_DIR",
    "COOKIE_SECURE",
    "AUTH0_ROLES_CLAIM",
    "SESSION_SECRET",
    "AUTH0_DOMAIN",
    "AUTH0_CLIENT_ID",
    "AUTH0_CLIENT_SECRET",
];

fn clear_env() {
    for k in ENV_KEYS {
        // SAFETY: tests holding ENV_LOCK are the only ones mutating these vars.
        unsafe { std::env::remove_var(k) };
    }
}

fn set_env(k: &str, v: &str) {
    unsafe { std::env::set_var(k, v) };
}

#[actix_web::test]
async fn config_defaults() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    let c = Config::from_env().unwrap();
    assert_eq!(c.bind_address, "127.0.0.1:8080");
    assert_eq!(c.base_url, "http://localhost:8080");
    assert_eq!(c.static_dir, "crates/web/static");
    assert!(!c.cookie_secure);
    assert_eq!(c.roles_claim, "https://amateur-radio-tools/roles");
    assert!(c.session_secret.is_none());
    assert!(c.auth0.is_none());
    assert_eq!(c.redirect_uri(), "http://localhost:8080/auth/callback");
    assert!(c.summary().contains("auth0=<disabled>"));
}

#[actix_web::test]
async fn config_session_secret_rules() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    set_env("SESSION_SECRET", "too-short");
    assert!(Config::from_env().is_err());

    clear_env();
    let long = "a".repeat(64);
    set_env("SESSION_SECRET", &long);
    assert_eq!(
        Config::from_env().unwrap().session_secret.as_deref(),
        Some(long.as_str())
    );
    clear_env();
}

#[actix_web::test]
async fn config_auth0_and_bools() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    // Only a domain ⇒ Auth0 stays disabled (all-or-nothing).
    set_env("AUTH0_DOMAIN", "tenant.us.auth0.com");
    assert!(Config::from_env().unwrap().auth0.is_none());

    // All three ⇒ enabled, domain normalized (scheme + trailing slash stripped).
    set_env("AUTH0_DOMAIN", "https://tenant.us.auth0.com/");
    set_env("AUTH0_CLIENT_ID", "cid");
    set_env("AUTH0_CLIENT_SECRET", "secret");
    set_env("COOKIE_SECURE", "yes");
    let c = Config::from_env().unwrap();
    let a = c.auth0.expect("auth0 enabled");
    assert_eq!(a.domain, "tenant.us.auth0.com");
    assert!(c.cookie_secure);

    clear_env();
    set_env("COOKIE_SECURE", "0");
    assert!(!Config::from_env().unwrap().cookie_secure);
    clear_env();
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

#[actix_web::test]
async fn app_error_status_codes() {
    use actix_web::http::StatusCode;
    assert_eq!(AppError::NotFound.status_code(), StatusCode::NOT_FOUND);
    assert_eq!(AppError::Forbidden.status_code(), StatusCode::FORBIDDEN);
    assert_eq!(
        AppError::BadRequest("x".into()).status_code(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        AppError::AuthNotConfigured.status_code(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        AppError::Auth("x".into()).status_code(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        AppError::Internal(anyhow::anyhow!("boom")).status_code(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    // From<DbErr>
    let e: AppError = sea_orm::DbErr::Custom("db".into()).into();
    assert_eq!(e.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[actix_web::test]
async fn app_error_renders_html() {
    let resp = AppError::NotFound.error_response();
    assert_eq!(resp.status(), 404);
    let ct = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(ct.contains("text/html"));
    let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("404"));
}

#[actix_web::test]
async fn api_error_renders_json() {
    use actix_web::http::StatusCode;
    assert_eq!(
        ApiError::Unauthorized.status_code(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        ApiError::BadRequest("x".into()).status_code(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(ApiError::NotFound.status_code(), StatusCode::NOT_FOUND);
    assert_eq!(
        ApiError::internal("x").status_code(),
        StatusCode::INTERNAL_SERVER_ERROR
    );

    let resp = ApiError::Unauthorized.error_response();
    assert_eq!(resp.status(), 401);
    assert!(resp.headers().contains_key(header::WWW_AUTHENTICATE));
    let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "unauthorized");

    // From<DbErr> maps to internal.
    let e: ApiError = sea_orm::DbErr::Custom("db".into()).into();
    assert_eq!(e.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

// ---------------------------------------------------------------------------
// Session helpers
// ---------------------------------------------------------------------------

#[actix_web::test]
async fn session_user_helpers() {
    let named = session_user(1, vec!["Admin".to_string()]);
    assert_eq!(named.display_name(), "Test User");
    assert_eq!(named.initials(), "TU");
    assert!(named.has_role("admin")); // case-insensitive
    assert!(named.is_admin());

    let no_name = SessionUser {
        id: 2,
        sub: "s".into(),
        email: "solo@example.com".into(),
        name: None,
        picture: None,
        roles: vec![],
    };
    assert_eq!(no_name.display_name(), "solo@example.com");
    assert_eq!(no_name.initials(), "S");
    assert!(!no_name.is_admin());
}
