//! Shared application state, injected into every handler as `web::Data<AppState>`.

use crate::auth::oidc::AuthClient;
use crate::config::Config;
use sea_orm::DatabaseConnection;

pub struct AppState {
    pub db: DatabaseConnection,
    pub config: Config,
    /// The Auth0 OIDC client. `None` when authentication is not configured.
    pub auth: Option<AuthClient>,
}
