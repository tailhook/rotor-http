//! HTTP Client implementation
//!
//! Currently we aim to provide only HTTP/1.x implementation. We want to
//! provide HTTP/2.0 and TLS implementation with exactly the same protocol.
//! But it's yet unproven if it is possible.
//!
//! Also DNS resolving is not implemented yet.

mod context;
mod pool;
mod request;
mod creator;
mod response;

pub use self::context::{Context, Pool};
pub use self::request::{Request};
pub use self::creator::{RequestCreator};
pub use self::pool::{PipeInit, PoolFsm};
pub use self::response::{Response};

use rotor::Peer1Socket;

/// Token useed to initialize Pool state machine
pub struct PoolFsmInit<R: RequestCreator>(Peer1Socket<pool::Pool<R>>);
