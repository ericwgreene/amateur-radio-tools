//! Add the operator's own station identity to `users`.
//!
//! `sessions` already records the callsign and grid used for a particular run, which is
//! what portable, POTA, and club-call operating need. But there has to be a default to
//! fall back on, and a future ADIF export needs a stable `STATION_CALLSIGN` /
//! `MY_GRIDSQUARE` anchor that isn't tied to one session. So both exist: the session
//! value wins when set, the user value is the default.
//!
//! Nullable and unvalidated on purpose — `users` is otherwise a mirror of Auth0, and a
//! user who never touches the setting should be unaffected.
//!
//! Each column is added in its own `ALTER` statement: SQLite rejects a statement
//! carrying more than one alter option, so batching them would work on PostgreSQL and
//! panic on the SQLite path used by the tests.

use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Users::Table)
                    .add_column(string_null(Users::Callsign))
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Users::Table)
                    .add_column(string_null(Users::Grid))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop only what this migration added — the table itself belongs to
        // `m20250722_000001_create_users`.
        manager
            .alter_table(
                Table::alter()
                    .table(Users::Table)
                    .drop_column(Users::Callsign)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Users::Table)
                    .drop_column(Users::Grid)
                    .to_owned(),
            )
            .await
    }
}

/// Column identifiers; kept in sync with `entity::users::Model`.
#[derive(DeriveIden)]
enum Users {
    Table,
    Callsign,
    Grid,
}
