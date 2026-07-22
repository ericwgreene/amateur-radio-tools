//! Standalone migration runner.
//!
//! Usage (reads DATABASE_URL from the environment):
//!   cargo run -p migration -- up          # apply all pending migrations
//!   cargo run -p migration -- down        # revert the last migration
//!   cargo run -p migration -- status      # show migration status
//!   cargo run -p migration -- fresh       # drop all tables and re-apply
//!
//! The `web` binary also applies pending migrations automatically on startup, so this
//! runner is mainly for local development and CI.

use sea_orm_migration::prelude::*;

#[tokio::main]
async fn main() {
    cli::run_cli(migration::Migrator).await;
}
