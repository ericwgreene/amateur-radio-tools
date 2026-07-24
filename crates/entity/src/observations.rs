//! The `observations` entity — one heard transmission.
//!
//! Distinct from `contacts` by design: a contact is a QSO (two stations worked each
//! other), an observation is one-way (a station was heard). Keeping them apart is what
//! lets the logbook stay a real logbook while the monitoring log grows freely.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "observations")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// Owner (`users.id`).
    #[sea_orm(indexed)]
    pub user_id: i64,
    /// Parent session (`sessions.id`), resolved server-side from the session's
    /// `client_key`.
    pub session_id: i64,
    /// Client-generated identifier, unique per user. A batch replayed after a network
    /// failure inserts nothing the second time.
    pub client_key: String,
    /// The callsign heard (stored uppercased). Never empty — unidentified transmissions
    /// are not uploaded.
    pub callsign: String,
    /// When the transmission started (UTC).
    pub heard_at: DateTimeUtc,
    /// How long the transmission lasted.
    pub duration_secs: Option<f64>,
    /// Snapshotted from the session at ingest, so editing the session later does not
    /// rewrite history.
    pub band: Option<String>,
    pub mode: Option<String>,
    pub frequency_mhz: Option<f64>,
    /// DXCC entity resolved from the callsign at ingest time.
    pub country: Option<String>,
    /// Transcribed audio. Only present when the operator explicitly opted in to
    /// uploading transcript text; the default is off.
    pub transcript: Option<String>,
    pub source: Option<String>,
    /// Set once this observation has been promoted into a logbook contact.
    pub promoted_contact_id: Option<i64>,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
