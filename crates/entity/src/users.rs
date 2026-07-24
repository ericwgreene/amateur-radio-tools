//! The `users` entity.
//!
//! Auth0 is the source of truth for *identity* (authentication). This table is a local
//! mirror of the users who have logged in, keyed by the Auth0 subject (`sub`) claim. It
//! gives the application a stable local primary key to hang app-specific data off of
//! (logbooks, saved tools, preferences, ...) without round-tripping to Auth0.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// The Auth0 `sub` claim, e.g. `auth0|abc123` or `google-oauth2|...`. Unique.
    #[sea_orm(unique, indexed)]
    pub auth0_sub: String,
    pub email: String,
    pub name: Option<String>,
    pub picture: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub last_login_at: Option<DateTimeUtc>,
    /// The operator's own callsign. A per-session value overrides this; this is the
    /// default, and the anchor a future ADIF export needs for `STATION_CALLSIGN`.
    /// Appended here to match the column order the ALTER produces.
    pub callsign: Option<String>,
    /// The operator's own Maidenhead grid, on the same default-and-override footing.
    pub grid: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
