//! Create the `stations` table — the unique-callsign rollup.
//!
//! One row per `(user, callsign)`: the log of every distinct station this operator has
//! ever heard, with first/last heard timestamps, a hearing count, and the licensee
//! details cached from whatever lookup the client performed. The unique index on
//! `(user_id, callsign)` *is* the "unique contacts over time" guarantee.
//!
//! This is a materialized rollup rather than a `GROUP BY` view for two reasons: the
//! roster is read far more often than it is written, and — more importantly — it gives
//! the operator's own notes somewhere to live, and gives the desktop app something to
//! read back at startup so a station heard last month resolves instantly instead of
//! re-fetching from the FCC.
//!
//! Note what is deliberately *absent*: a `times_worked` counter. Contacts are created
//! from several independent paths, none of which would know to bump it, so a stored
//! counter would read zero for stations worked through the ordinary logbook. It is
//! derived from `contacts` at read time instead, where it cannot drift.

use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Stations::Table)
                    .if_not_exists()
                    .col(pk_auto(Stations::Id))
                    .col(big_integer(Stations::UserId))
                    .col(string(Stations::Callsign))
                    .col(timestamp_with_time_zone(Stations::FirstHeardAt))
                    .col(timestamp_with_time_zone(Stations::LastHeardAt))
                    .col(big_integer(Stations::TimesHeard))
                    .col(string_null(Stations::Name))
                    .col(string_null(Stations::Qth))
                    .col(string_null(Stations::Grid))
                    .col(string_null(Stations::Country))
                    .col(text_null(Stations::Notes))
                    .col(timestamp_with_time_zone(Stations::CreatedAt))
                    .col(timestamp_with_time_zone(Stations::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        // This constraint is the feature: one row per station per operator.
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("uq_stations_user_callsign")
                    .table(Stations::Table)
                    .col(Stations::UserId)
                    .col(Stations::Callsign)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // The two orderings the roster page offers.
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_stations_user_last_heard_at")
                    .table(Stations::Table)
                    .col(Stations::UserId)
                    .col(Stations::LastHeardAt)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_stations_user_times_heard")
                    .table(Stations::Table)
                    .col(Stations::UserId)
                    .col(Stations::TimesHeard)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Stations::Table).to_owned())
            .await
    }
}

/// Column identifiers; kept in sync with `entity::stations::Model`.
#[derive(DeriveIden)]
enum Stations {
    Table,
    Id,
    UserId,
    Callsign,
    FirstHeardAt,
    LastHeardAt,
    TimesHeard,
    Name,
    Qth,
    Grid,
    Country,
    Notes,
    CreatedAt,
    UpdatedAt,
}
