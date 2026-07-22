//! The `contacts` entity — a logbook entry (a "QSO", in ham parlance).
//!
//! Each row is one logged contact belonging to a user. Kept deliberately flat and simple;
//! this is a foundation to grow (ADIF import/export, per-band stats, awards, ...).

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "contacts")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// Owner (`users.id`).
    #[sea_orm(indexed)]
    pub user_id: i64,
    /// The callsign of the station worked (stored uppercased).
    pub callsign: String,
    /// When the contact took place (UTC).
    pub worked_at: DateTimeUtc,
    pub band: Option<String>,
    pub mode: Option<String>,
    pub frequency_mhz: Option<f64>,
    pub rst_sent: Option<String>,
    pub rst_received: Option<String>,
    /// The other station's Maidenhead grid locator.
    pub grid: Option<String>,
    /// Operator name.
    pub name: Option<String>,
    /// Location / QTH.
    pub qth: Option<String>,
    /// DXCC entity resolved from the callsign at logging time.
    pub country: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
