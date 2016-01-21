//! HTTP Client implementation
//!
//! Currently we aim to provide only HTTP/1.x implementation. We want to
//! provide HTTP/2.0 and TLS implementation with exactly the same protocol.
//! But it's yet unproven if it is possible.
//!
//! Also DNS resolving is not implemented yet.
//!

use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use rotor::EarlyScope;

mod pool;
mod request;
mod fsm;
mod head;
mod protocol;
mod api;

pub use self::request::{Request};
pub use self::protocol::Client;
pub use self::head::Head;
pub use recvmode::RecvMode;

/// The internal connection pool's state machine
///
/// Must be used as a return value of `add_machine_with`. No direct usage
/// of it is expected.
pub struct PoolFsm<R: Client>(Arc<Mutex<pool::Pool<R>>>);

/// The connection pool
///
/// The object may be used to initiate a connection from any state machine
/// and also be used from other threads
#[derive(Clone)]
pub struct Pool<R: Client>(Arc<Mutex<pool::Pool<R>>>);


/// Create a socket pool
///
/// This is expected to be called from `LoopCreator::add_machine_with`.
/// The first element of returned tuple is a state machine that should be
/// returned from the lambda.
///
/// The second element of the tuple is a pool object that might be used to
/// initiate a connection from any state machine or from other thread.
pub fn create_pool<R: Client>(scope: &mut EarlyScope) -> (PoolFsm<R>, Pool<R>)
{
    let internal = pool::Pool::new(scope.notifier());
    let arc = Arc::new(Mutex::new(internal));
    return (PoolFsm(arc.clone()), Pool(arc.clone()));
}
