use std::marker::PhantomData;
use rotor::{Peer2Socket, Peer1Socket, Scope, create_pair};
use super::{pool, RequestCreator, PipeInit, PoolFsmInit, Response};

/// A pool of HTTP connections
pub struct Pool<R: RequestCreator>(Peer2Socket<pool::Pool<R>>);

pub trait Context<R: RequestCreator> {
    fn get_pool<'x>(&'x mut self) -> &'x mut Pool<R>;
}

impl<R: RequestCreator> Pool<R> {
    /// Creates a pool
    ///
    /// The method returns a Pool and some kind of token, that may be used
    /// to initialize the state machine of the connection.
    pub fn create() -> (PoolFsmInit<R>, Pool<R>)
    {
        let (tok1, tok2) = create_pair(pool::Pool::new());
        (PoolFsmInit(tok1), Pool(tok2))
    }
}
