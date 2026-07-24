//! The `sessions` entity — one operating or monitoring run.
//!
//! Groups the transmissions heard during a single stretch of operating, and carries the
//! radio metadata a receive-only monitor cannot discover for itself.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "sessions")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// Owner (`users.id`).
    #[sea_orm(indexed)]
    pub user_id: i64,
    /// Client-generated identifier, unique per user. Lets a desktop app open and update
    /// a session without ever learning the server-assigned `id` — which is what makes
    /// fully-offline operation possible.
    pub client_key: String,
    /// `monitor` | `net` | `contest` | `pota`. Validated in the handler, not the schema.
    pub kind: String,
    /// Free-text label, e.g. "Tuesday ARES net".
    pub label: Option<String>,
    pub started_at: DateTimeUtc,
    /// `None` while the session is still running.
    pub ended_at: Option<DateTimeUtc>,
    pub band: Option<String>,
    pub mode: Option<String>,
    pub frequency_mhz: Option<f64>,
    /// The callsign operated for this run; falls back to `users.callsign` when unset.
    pub operator_callsign: Option<String>,
    /// The operator's grid for this run; falls back to `users.grid` when unset.
    pub grid: Option<String>,
    /// Which application recorded this, e.g. `radio-monitor/0.3.0`.
    pub source: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
