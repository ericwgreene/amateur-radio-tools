//! Database entities shared across the workspace.
//!
//! Entities are intentionally kept in their own crate (SeaORM's recommended layout)
//! so both the `migration` crate and the `web` application can depend on them without
//! creating a dependency cycle, and so entities can be regenerated with `sea-orm-cli`.

pub mod api_tokens;
pub mod contacts;
pub mod observations;
pub mod prelude;
pub mod sessions;
pub mod stations;
pub mod users;
