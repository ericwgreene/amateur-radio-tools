//! Create the `observations` table — one heard transmission.
//!
//! An observation is deliberately *not* a `contacts` row. A contact is a QSO: two
//! stations worked each other, and RST reports were exchanged. An observation is
//! one-way — a station was heard on the air. Conflating the two would make a future
//! ADIF export meaningless and would bury a real logbook under the dozens of hearings
//! per hour that a busy net produces. An observation can be *promoted* into a contact
//! once the operator actually works the station; `promoted_contact_id` records that.
//!
//! `band`, `mode`, and `frequency_mhz` are snapshotted from the parent session at ingest
//! rather than read through it. A session's frequency can change mid-run, and history
//! should not be rewritten when it does.

use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Observations::Table)
                    .if_not_exists()
                    .col(pk_auto(Observations::Id))
                    .col(big_integer(Observations::UserId))
                    .col(big_integer(Observations::SessionId))
                    .col(string(Observations::ClientKey))
                    // Not nullable: unidentified transmissions are never uploaded, so
                    // every row here has a callsign by construction.
                    .col(string(Observations::Callsign))
                    .col(timestamp_with_time_zone(Observations::HeardAt))
                    .col(double_null(Observations::DurationSecs))
                    .col(string_null(Observations::Band))
                    .col(string_null(Observations::Mode))
                    .col(double_null(Observations::FrequencyMhz))
                    .col(string_null(Observations::Country))
                    // Only populated when the operator explicitly opts in to uploading
                    // transcript text; the default is off.
                    .col(text_null(Observations::Transcript))
                    .col(string_null(Observations::Source))
                    .col(big_integer_null(Observations::PromotedContactId))
                    .col(timestamp_with_time_zone(Observations::CreatedAt))
                    .to_owned(),
            )
            .await?;

        // The monitoring log is always "this user, newest first".
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_observations_user_heard_at")
                    .table(Observations::Table)
                    .col(Observations::UserId)
                    .col(Observations::HeardAt)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_observations_session_id")
                    .table(Observations::Table)
                    .col(Observations::SessionId)
                    .to_owned(),
            )
            .await?;

        // Drives the per-station detail page.
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_observations_user_callsign")
                    .table(Observations::Table)
                    .col(Observations::UserId)
                    .col(Observations::Callsign)
                    .to_owned(),
            )
            .await?;

        // The idempotency guarantee: a batch replayed after a network failure inserts
        // nothing the second time.
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("uq_observations_user_client_key")
                    .table(Observations::Table)
                    .col(Observations::UserId)
                    .col(Observations::ClientKey)
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Observations::Table).to_owned())
            .await
    }
}

/// Column identifiers; kept in sync with `entity::observations::Model`.
#[derive(DeriveIden)]
enum Observations {
    Table,
    Id,
    UserId,
    SessionId,
    ClientKey,
    Callsign,
    HeardAt,
    DurationSecs,
    Band,
    Mode,
    FrequencyMhz,
    Country,
    Transcript,
    Source,
    PromotedContactId,
    CreatedAt,
}
