//! Database logic shared between the JSON API and the browser routes.
//!
//! This sits between `tools/` (pure, offline domain logic that never touches a
//! database) and the handler modules (which own HTTP concerns). Everything here takes a
//! connection or a transaction and returns `Result<_, DbErr>` — deliberately *not*
//! `ApiError` or `AppError`, so the same function serves a JSON endpoint and an HTMX
//! fragment without either error type leaking into the other's surface, and so the
//! logic can be unit-tested directly against `test_db()`.

pub mod observations;
pub mod sessions;
pub mod stations;
