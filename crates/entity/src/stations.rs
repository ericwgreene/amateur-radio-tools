//! The `stations` entity — the unique-callsign rollup.
//!
//! One row per `(user, callsign)`: every distinct station this operator has ever heard.
//! Maintained incrementally as observations arrive rather than computed on demand, so
//! it has somewhere to keep the operator's own notes and can be read back by a desktop
//! client to pre-warm its callsign cache.
//!
//! There is no `times_worked` column — see the migration for why it is derived from
//! `contacts` instead of stored.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "stations")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// Owner (`users.id`).
    #[sea_orm(indexed)]
    pub user_id: i64,
    /// The station's callsign (stored uppercased). Unique within a user.
    pub callsign: String,
    /// Earliest hearing. Only ever moves earlier.
    pub first_heard_at: DateTimeUtc,
    /// Latest hearing. Only ever moves later — a batch uploaded days late carries old
    /// timestamps and must not drag this backwards.
    pub last_heard_at: DateTimeUtc,
    pub times_heard: i64,
    /// Licensee details, cached from whatever lookup the client performed. Filled in
    /// opportunistically and never overwritten with a blank.
    pub name: Option<String>,
    pub qth: Option<String>,
    pub grid: Option<String>,
    pub country: Option<String>,
    /// The operator's own notes about this station.
    pub notes: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
