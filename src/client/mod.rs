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
mod creator;
mod response;
mod fsm;

pub use self::request::{Request};
pub use self::creator::{RequestCreator};
pub use self::response::{Response};

pub struct PoolFsm<R, C>(Arc<Mutex<pool::Pool<R>>>, PhantomData<*const C>)
    where R: RequestCreator;
pub struct Pool<R: RequestCreator>(Arc<Mutex<pool::Pool<R>>>);


pub fn create_pool<R: RequestCreator, C>(scope: &mut EarlyScope)
    -> (PoolFsm<R, C>, Pool<R>)
{
    let internal = pool::Pool::new(scope.notifier());
    let arc = Arc::new(Mutex::new(internal));
    return (PoolFsm(arc.clone(), PhantomData), Pool(arc.clone()));
}
