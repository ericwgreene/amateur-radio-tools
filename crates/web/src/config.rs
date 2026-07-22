//! Application configuration, loaded from the environment.
//!
//! Every setting has a sensible default so the app boots with zero configuration for
//! local development. Auth0 settings are optional: if they are absent the app still runs,
//! with authentication disabled, so you can work on public pages without a tenant.

use anyhow::{Result, bail};

#[derive(Clone, Debug)]
pub struct Config {
    /// Socket address to bind, e.g. `127.0.0.1:8080`.
    pub bind_address: String,
    /// Public base URL used to build absolute redirect/callback URLs, e.g.
    /// `http://localhost:8080`. Must match what is registered in Auth0.
    pub base_url: String,
    /// SeaORM database URL. `sqlite://...` or `postgres://...`.
    pub database_url: String,
    /// Directory served at `/static`.
    pub static_dir: String,
    /// 64+ byte secret used to sign/encrypt the session cookie. If unset an ephemeral
    /// key is generated at startup (fine for dev; sessions won't survive a restart).
    pub session_secret: Option<String>,
    /// Whether the session cookie requires HTTPS. Must be `true` in production; defaults
    /// to `false` so cookies work over plain `http://localhost` in development.
    pub cookie_secure: bool,
    /// The namespaced ID-token claim Auth0 uses to deliver roles (see README).
    pub roles_claim: String,
    /// Auth0 credentials. `None` disables authentication.
    pub auth0: Option<Auth0Config>,
}

#[derive(Clone, Debug)]
pub struct Auth0Config {
    /// Tenant domain, without scheme, e.g. `your-tenant.us.auth0.com`.
    pub domain: String,
    pub client_id: String,
    pub client_secret: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let bind_address = env_or("BIND_ADDRESS", "127.0.0.1:8080");
        let base_url = env_or("BASE_URL", "http://localhost:8080");
        let database_url = env_or(
            "DATABASE_URL",
            "sqlite://./data/amateur_radio_tools.db?mode=rwc",
        );
        let static_dir = env_or("STATIC_DIR", "crates/web/static");
        let cookie_secure = env_bool("COOKIE_SECURE", false);
        let roles_claim = env_or("AUTH0_ROLES_CLAIM", "https://amateur-radio-tools/roles");

        let session_secret = match std::env::var("SESSION_SECRET") {
            Ok(s) if s.is_empty() => None,
            Ok(s) if s.len() < 64 => {
                bail!(
                    "SESSION_SECRET must be at least 64 bytes (generate one with `openssl rand -hex 64`)"
                )
            }
            Ok(s) => Some(s),
            Err(_) => None,
        };

        let auth0 = match (
            non_empty("AUTH0_DOMAIN"),
            non_empty("AUTH0_CLIENT_ID"),
            non_empty("AUTH0_CLIENT_SECRET"),
        ) {
            (Some(domain), Some(client_id), Some(client_secret)) => Some(Auth0Config {
                // Accept a full URL or a bare domain and normalize to a bare host.
                domain: domain
                    .trim()
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .trim_end_matches('/')
                    .to_string(),
                client_id,
                client_secret,
            }),
            _ => None,
        };

        Ok(Self {
            bind_address,
            base_url,
            database_url,
            static_dir,
            session_secret,
            cookie_secure,
            roles_claim,
            auth0,
        })
    }

    /// The OAuth2 redirect/callback URL registered in Auth0.
    pub fn redirect_uri(&self) -> String {
        format!("{}/auth/callback", self.base_url.trim_end_matches('/'))
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn non_empty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => default,
    }
}

impl Config {
    /// A best-effort context line for logs (never prints secrets).
    pub fn summary(&self) -> String {
        format!(
            "bind={} base_url={} db={} auth0={}",
            self.bind_address,
            self.base_url,
            scheme_of(&self.database_url),
            self.auth0
                .as_ref()
                .map(|a| a.domain.as_str())
                .unwrap_or("<disabled>"),
        )
    }
}

fn scheme_of(url: &str) -> &str {
    url.split(':').next().unwrap_or("unknown")
}
