//! Database migrations.
//!
//! Migrations are written against SeaORM's schema builder rather than raw SQL, which
//! keeps them portable across backends. The same migration set applies whether
//! `DATABASE_URL` points at SQLite or PostgreSQL.

pub use sea_orm_migration::prelude::*;

mod m20250722_000001_create_users;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20250722_000001_create_users::Migration)]
    }
}
