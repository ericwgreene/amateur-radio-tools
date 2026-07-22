//! Shared test fixtures.
//!
//! The web handlers are exercised end-to-end against an **in-memory SQLite** database
//! (the same `Migrator` used in production) driven through `actix_web::test`. This keeps
//! the whole suite offline — no PostgreSQL, no network, no Auth0 — while still covering the
//! real request → extractor → handler → DB → template path.

use std::collections::HashMap;

use actix_session::Session;
use actix_web::{HttpResponse, web};
use chrono::Utc;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, ConnectOptions, Database, DatabaseConnection, Set};

use crate::auth::api_token::generate_token;
use crate::auth::session::{SESSION_USER_KEY, SessionUser};
use crate::config::Config;
use crate::state::AppState;
use entity::{api_tokens, contacts, users};

/// Fixed 64-byte session key so cookies round-trip within a test app.
pub(crate) const TEST_KEY: [u8; 64] = [7u8; 64];
/// Session cookie name (matches production for realism).
pub(crate) const SESSION_COOKIE: &str = "art_session";

/// A fresh in-memory database with all migrations applied.
///
/// The pool is pinned to a single connection: a multi-connection `sqlite::memory:` pool
/// hands each connection its own empty database, which would make the migrated schema
/// disappear between statements.
pub(crate) async fn test_db() -> DatabaseConnection {
    let mut opts = ConnectOptions::new("sqlite::memory:");
    opts.max_connections(1).min_connections(1);
    let db = Database::connect(opts)
        .await
        .expect("connect in-memory sqlite");
    Migrator::up(&db, None).await.expect("run migrations");
    db
}

/// A minimal `Config` with authentication disabled.
pub(crate) fn test_config() -> Config {
    Config {
        bind_address: "127.0.0.1:0".to_string(),
        base_url: "http://localhost:8080".to_string(),
        database_url: "sqlite::memory:".to_string(),
        static_dir: "crates/web/static".to_string(),
        session_secret: None,
        cookie_secure: false,
        roles_claim: "https://amateur-radio-tools/roles".to_string(),
        auth0: None,
    }
}

pub(crate) async fn test_state() -> web::Data<AppState> {
    web::Data::new(AppState {
        db: test_db().await,
        config: test_config(),
        auth: None,
    })
}

/// Insert a user and return it.
pub(crate) async fn seed_user(db: &DatabaseConnection, email: &str) -> users::Model {
    let now = Utc::now();
    users::ActiveModel {
        auth0_sub: Set(format!("test|{email}")),
        email: Set(email.to_string()),
        name: Set(Some("Test User".to_string())),
        picture: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        last_login_at: Set(Some(now)),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("seed user")
}

/// Mint an API token for a user; returns `(raw_token, token_id)`.
pub(crate) async fn seed_token(db: &DatabaseConnection, user_id: i64) -> (String, i64) {
    let token = generate_token();
    let now = Utc::now();
    let model = api_tokens::ActiveModel {
        user_id: Set(user_id),
        name: Set("test token".to_string()),
        token_hash: Set(token.hash),
        token_prefix: Set(token.prefix),
        created_at: Set(now),
        last_used_at: Set(None),
        revoked_at: Set(None),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("seed token");
    (token.raw, model.id)
}

/// Insert a logbook contact for a user and return it.
pub(crate) async fn seed_contact(
    db: &DatabaseConnection,
    user_id: i64,
    callsign: &str,
) -> contacts::Model {
    let now = Utc::now();
    contacts::ActiveModel {
        user_id: Set(user_id),
        callsign: Set(callsign.to_string()),
        worked_at: Set(now),
        band: Set(None),
        mode: Set(None),
        frequency_mhz: Set(None),
        rst_sent: Set(None),
        rst_received: Set(None),
        grid: Set(None),
        name: Set(None),
        qth: Set(None),
        country: Set(None),
        notes: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("seed contact")
}

/// A `SessionUser` for direct (non-HTTP) handler/extractor tests.
pub(crate) fn session_user(id: i64, roles: Vec<String>) -> SessionUser {
    SessionUser {
        id,
        sub: "test|sub".to_string(),
        email: "test@example.com".to_string(),
        name: Some("Test User".to_string()),
        picture: None,
        roles,
    }
}

/// Test-only route (`POST /__test_login/{id}?admin=bool`) that establishes a session so
/// cookie-authenticated handlers can be exercised. Only ever mounted by `test_app!`.
pub(crate) async fn test_login(
    session: Session,
    path: web::Path<i64>,
    query: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    let admin = query.get("admin").map(|v| v == "true").unwrap_or(false);
    let roles = if admin {
        vec!["admin".to_string()]
    } else {
        Vec::new()
    };
    let user = session_user(path.into_inner(), roles);
    session
        .insert(SESSION_USER_KEY, &user)
        .expect("insert session");
    HttpResponse::Ok().finish()
}

/// Build an initialized test service: full route table + session middleware + the test
/// login route. Returns what `actix_web::test::init_service` returns (type left inferred).
macro_rules! test_app {
    ($state:expr) => {{
        actix_web::test::init_service(
            actix_web::App::new()
                .app_data($state.clone())
                .wrap(
                    actix_session::SessionMiddleware::builder(
                        actix_session::storage::CookieSessionStore::default(),
                        actix_web::cookie::Key::from(&$crate::test_support::TEST_KEY),
                    )
                    .cookie_name($crate::test_support::SESSION_COOKIE.to_string())
                    .cookie_secure(false)
                    .build(),
                )
                .route(
                    "/__test_login/{id}",
                    actix_web::web::post().to($crate::test_support::test_login),
                )
                .configure($crate::routes::configure),
        )
        .await
    }};
}
pub(crate) use test_app;

/// Log in against a test app and return the session cookie to replay on later requests.
macro_rules! login {
    ($app:expr, $id:expr, $admin:expr) => {{
        let uri = format!("/__test_login/{}?admin={}", $id, $admin);
        let req = actix_web::test::TestRequest::post().uri(&uri).to_request();
        let resp = actix_web::test::call_service(&$app, req).await;
        assert!(resp.status().is_success(), "test login failed");
        resp.response()
            .cookies()
            .find(|c| c.name() == $crate::test_support::SESSION_COOKIE)
            .expect("session cookie present")
            .into_owned()
    }};
}
pub(crate) use login;
