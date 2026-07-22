//! Create the `users` table.

use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Users::Table)
                    .if_not_exists()
                    .col(pk_auto(Users::Id))
                    .col(string_uniq(Users::Auth0Sub))
                    .col(string(Users::Email))
                    .col(string_null(Users::Name))
                    .col(string_null(Users::Picture))
                    .col(timestamp_with_time_zone(Users::CreatedAt))
                    .col(timestamp_with_time_zone(Users::UpdatedAt))
                    .col(timestamp_with_time_zone_null(Users::LastLoginAt))
                    .to_owned(),
            )
            .await
        // Note: `string_uniq` already creates a UNIQUE constraint on `auth0_sub`, which
        // is backed by an index on every supported backend, so no extra index is needed.
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Users::Table).to_owned())
            .await
    }
}

/// Column identifiers for the `users` table. Kept in sync with `entity::users::Model`.
#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Auth0Sub,
    Email,
    Name,
    Picture,
    CreatedAt,
    UpdatedAt,
    LastLoginAt,
}
