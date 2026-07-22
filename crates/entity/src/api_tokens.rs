//! The `api_tokens` entity — personal access tokens for the REST API.
//!
//! A token lets an external application (a logging program, a station computer, a script)
//! authenticate as a user without going through the browser OIDC flow. Only a SHA-256
//! **hash** of the token is stored; the raw token is shown to the user exactly once at
//! creation time and is unrecoverable afterwards.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "api_tokens")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// Owner (`users.id`).
    #[sea_orm(indexed)]
    pub user_id: i64,
    /// Human-friendly label, e.g. "WSJT-X on the shack PC".
    pub name: String,
    /// SHA-256 hex of the raw token. Never serialize this.
    #[serde(skip_serializing)]
    #[sea_orm(unique)]
    pub token_hash: String,
    /// A short, non-secret prefix of the token, for display (e.g. `art_a1b2c3`).
    pub token_prefix: String,
    pub created_at: DateTimeUtc,
    pub last_used_at: Option<DateTimeUtc>,
    /// Set when the token is revoked; a revoked token no longer authenticates.
    pub revoked_at: Option<DateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
