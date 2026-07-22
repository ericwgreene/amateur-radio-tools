//! Amateur Radio Tools — an Actix Web application, server-rendered with Askama and
//! progressively enhanced with HTMX, using Auth0 (OIDC) for authentication.

mod auth;
mod config;
mod error;
mod routes;
mod state;
mod tools;

use actix_files::Files;
use actix_session::{
    SessionMiddleware,
    config::{CookieContentSecurity, PersistentSession},
    storage::CookieSessionStore,
};
use actix_web::{App, HttpServer, cookie::Key, cookie::time::Duration, web};
use anyhow::{Context, Result};
use migration::{Migrator, MigratorTrait};
use sea_orm::Database;
use tracing_actix_web::TracingLogger;

use crate::auth::oidc::AuthClient;
use crate::config::Config;
use crate::state::AppState;

#[actix_web::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let config = Config::from_env()?;
    tracing::info!("configuration: {}", config.summary());

    ensure_sqlite_dir(&config.database_url);

    // Connect and bring the schema up to date. The migration set is backend-agnostic, so
    // this works identically against SQLite or PostgreSQL.
    let db = Database::connect(&config.database_url)
        .await
        .with_context(|| format!("failed to connect to database ({})", config.database_url))?;
    Migrator::up(&db, None)
        .await
        .context("failed to run migrations")?;
    tracing::info!("database ready, migrations applied");

    // A single shared HTTP client for OIDC. Redirects are disabled as a hardening measure
    // (openidconnect requires this to avoid SSRF-style issues during token exchange).
    let http = reqwest::ClientBuilder::new()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("failed to build HTTP client")?;

    let auth = match &config.auth0 {
        Some(a0) => match AuthClient::discover(a0, &config, http).await {
            Ok(client) => {
                tracing::info!("Auth0 OIDC configured for tenant {}", a0.domain);
                Some(client)
            }
            Err(e) => {
                tracing::error!("Auth0 discovery failed ({e:#}); authentication disabled");
                None
            }
        },
        None => {
            tracing::warn!("Auth0 not configured (set AUTH0_* env vars); authentication disabled");
            None
        }
    };

    let session_key = session_key(&config);
    let cookie_secure = config.cookie_secure;
    let static_dir = config.static_dir.clone();
    let bind_address = config.bind_address.clone();

    let state = web::Data::new(AppState { db, config, auth });

    tracing::info!("listening on http://{bind_address}");

    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .wrap(TracingLogger::default())
            .wrap(
                SessionMiddleware::builder(CookieSessionStore::default(), session_key.clone())
                    .cookie_name("art_session".to_string())
                    .cookie_secure(cookie_secure)
                    // Private = the cookie is encrypted and signed. We store the user's
                    // profile and roles in it, so encryption is appropriate.
                    .cookie_content_security(CookieContentSecurity::Private)
                    .session_lifecycle(PersistentSession::default().session_ttl(Duration::days(7)))
                    .build(),
            )
            .service(Files::new("/static", &static_dir).prefer_utf8(true))
            .configure(routes::configure)
    })
    .bind(&bind_address)
    .with_context(|| format!("failed to bind {bind_address}"))?
    .run()
    .await
    .context("server error")
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,web=debug,tower_http=info"));
    fmt().with_env_filter(filter).init();
}

/// Build the session cookie key from `SESSION_SECRET`, or generate an ephemeral one.
fn session_key(config: &Config) -> Key {
    match &config.session_secret {
        Some(secret) => Key::from(secret.as_bytes()),
        None => {
            tracing::warn!(
                "SESSION_SECRET not set — using an ephemeral key; sessions will not survive a restart"
            );
            Key::generate()
        }
    }
}

/// For a file-based SQLite URL, make sure the parent directory exists so the DB file can
/// be created on first run.
fn ensure_sqlite_dir(database_url: &str) {
    let Some(rest) = database_url.strip_prefix("sqlite://") else {
        return;
    };
    let path = rest.split('?').next().unwrap_or(rest);
    if path.is_empty() || path == ":memory:" {
        return;
    }
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
}
