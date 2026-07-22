//! Create the `api_tokens` table (personal access tokens for the REST API).

use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ApiTokens::Table)
                    .if_not_exists()
                    .col(pk_auto(ApiTokens::Id))
                    .col(big_integer(ApiTokens::UserId))
                    .col(string(ApiTokens::Name))
                    .col(string_uniq(ApiTokens::TokenHash))
                    .col(string(ApiTokens::TokenPrefix))
                    .col(timestamp_with_time_zone(ApiTokens::CreatedAt))
                    .col(timestamp_with_time_zone_null(ApiTokens::LastUsedAt))
                    .col(timestamp_with_time_zone_null(ApiTokens::RevokedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_api_tokens_user_id")
                    .table(ApiTokens::Table)
                    .col(ApiTokens::UserId)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ApiTokens::Table).to_owned())
            .await
    }
}

/// Column identifiers; kept in sync with `entity::api_tokens::Model`.
#[derive(DeriveIden)]
enum ApiTokens {
    Table,
    Id,
    UserId,
    Name,
    TokenHash,
    TokenPrefix,
    CreatedAt,
    LastUsedAt,
    RevokedAt,
}
