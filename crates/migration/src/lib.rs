//! Database migrations.
//!
//! Migrations are written against SeaORM's schema builder rather than raw SQL, which
//! keeps them portable across backends. The same migration set applies whether
//! `DATABASE_URL` points at SQLite or PostgreSQL.

pub use sea_orm_migration::prelude::*;

mod m20250722_000001_create_users;
mod m20250722_000002_create_contacts;
mod m20250722_000003_create_api_tokens;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20250722_000001_create_users::Migration),
            Box::new(m20250722_000002_create_contacts::Migration),
            Box::new(m20250722_000003_create_api_tokens::Migration),
        ]
    }
}
