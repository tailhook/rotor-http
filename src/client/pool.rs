use std::marker::PhantomData;
use std::collections::HashMap;

use ip::IpAddr;
use rotor::{Port, Scope, create_pair};
use rotor::{Monitor, Peer2Socket, Peer1Monitor, Peer1Socket};

use super::{RequestCreator, Context, PoolFsmInit};


/// Token that is used to initialize single http connection
pub struct PipeInit<R>(IpAddr, Peer2Socket<R>);

pub struct Chunk<R: RequestCreator> {
    idle: Vec<Peer2Socket<Peer2Socket<R>>>,
    busy: Vec<Peer2Socket<Peer2Socket<R>>>,
}

pub struct Pool<R: RequestCreator> {
    connections: HashMap<IpAddr, Chunk<R>>,
}

pub struct PoolFsm<R, C>(Peer1Monitor<Pool<R>>, PhantomData<*const C>)
    where R: RequestCreator, C: Context<R>;

impl<R: RequestCreator> Pool<R> {
    pub fn new() -> Pool<R> {
        Pool {
            connections: HashMap::new(),
        }
    }
}


impl<R: RequestCreator> PoolFsmInit<R>
{
    fn create<C: Context<R>>(self, scope: &mut Scope<C>) -> PoolFsm<R, C>
    {
        PoolFsm(self.0.initiate(scope), PhantomData)
    }
}
