//! Client identity and routing.

mod dispatcher;
mod id;

pub(crate) use dispatcher::ClientDispatcher;
pub use id::ClientId;
