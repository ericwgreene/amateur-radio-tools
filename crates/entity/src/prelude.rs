//! Convenient re-exports for entity consumers.

pub use super::api_tokens::{
    ActiveModel as ApiTokenActiveModel, Entity as ApiTokens, Model as ApiToken,
};
pub use super::contacts::{
    ActiveModel as ContactActiveModel, Entity as Contacts, Model as Contact,
};
pub use super::users::{ActiveModel as UserActiveModel, Entity as Users, Model as User};
