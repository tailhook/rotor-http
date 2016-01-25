//! HTTP Client implementation
//!
//! Currently we aim to provide only HTTP/1.x implementation. We want to
//! provide HTTP/2.0 and TLS implementation with exactly the same protocol.
//! But it's yet unproven if it is possible.
//!
//! Also DNS resolving is not implemented yet.
//!

use std::sync::{Arc, Mutex};

use rotor::EarlyScope;

mod pool;
mod request;
mod fsm;
mod head;
mod protocol;
mod api;
mod connection;
mod parser;
mod body;

pub use self::request::{Request};
pub use self::protocol::Client;
pub use self::head::Head;
pub use recvmode::RecvMode;

// TODO(tailhook) MAX_HEADERS_SIZE can be moved to Context
// (i.e. made non-constant), but it's more of a problem for MAX_HEADERS_NUM
// because that would mean we can't allocate array of headers on the stack
// so performance will degrade. Customizing MAX_HEADERS_SIZE is not very
// useful on it's own

/// Note httparse requires we preallocate array of this size so be wise
pub const MAX_HEADERS_NUM: usize = 256;
/// This one is not preallocated, but too large buffer is of limited use
/// because of previous parameter.
pub const MAX_HEADERS_SIZE: usize = 16384;
/// Maximum length of chunk size line. it would be okay with 12 bytes, but in
/// theory there might be some extensions which we probably should skip
///
/// Note: we don't have a limit on chunk body size. In buffered request mode
/// it's limited by either memory or artificial limit returned from handler.
/// In unbuffered mode we can process chunk of unlimited size as long as
/// request handler is able to handle it.
pub const MAX_CHUNK_HEAD: usize = 128;


/// The internal connection pool's state machine
///
/// Must be used as a return value of `add_machine_with`. No direct usage
/// of it is expected.
pub struct PoolFsm<R: Client>(fsm::Fsm<R>);

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
    return (PoolFsm(fsm::Fsm::pool(arc.clone())), Pool(arc.clone()));
}
