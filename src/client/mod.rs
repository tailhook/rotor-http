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

pub use self::request::{Request};
pub use self::protocol::Client;
pub use self::head::Head;
pub use recvmode::RecvMode;

pub struct PoolFsm<R: Client>(Arc<Mutex<pool::Pool<R>>>);
pub struct Pool<R: Client>(Arc<Mutex<pool::Pool<R>>>);


pub fn create_pool<R: Client>(scope: &mut EarlyScope) -> (PoolFsm<R>, Pool<R>)
{
    let internal = pool::Pool::new(scope.notifier());
    let arc = Arc::new(Mutex::new(internal));
    return (PoolFsm(arc.clone()), Pool(arc.clone()));
}
