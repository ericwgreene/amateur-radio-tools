//! Create the `contacts` (logbook) table.

use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Contacts::Table)
                    .if_not_exists()
                    .col(pk_auto(Contacts::Id))
                    .col(big_integer(Contacts::UserId))
                    .col(string(Contacts::Callsign))
                    .col(timestamp_with_time_zone(Contacts::WorkedAt))
                    .col(string_null(Contacts::Band))
                    .col(string_null(Contacts::Mode))
                    .col(double_null(Contacts::FrequencyMhz))
                    .col(string_null(Contacts::RstSent))
                    .col(string_null(Contacts::RstReceived))
                    .col(string_null(Contacts::Grid))
                    .col(string_null(Contacts::Name))
                    .col(string_null(Contacts::Qth))
                    .col(string_null(Contacts::Country))
                    .col(text_null(Contacts::Notes))
                    .col(timestamp_with_time_zone(Contacts::CreatedAt))
                    .col(timestamp_with_time_zone(Contacts::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_contacts_user_id")
                    .table(Contacts::Table)
                    .col(Contacts::UserId)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Contacts::Table).to_owned())
            .await
    }
}

/// Column identifiers; kept in sync with `entity::contacts::Model`.
#[derive(DeriveIden)]
enum Contacts {
    Table,
    Id,
    UserId,
    Callsign,
    WorkedAt,
    Band,
    Mode,
    FrequencyMhz,
    RstSent,
    RstReceived,
    Grid,
    Name,
    Qth,
    Country,
    Notes,
    CreatedAt,
    UpdatedAt,
}
