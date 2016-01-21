use std::sync::{Arc, Mutex};
use std::error::Error;

use rotor::{Machine, Scope, EventSet, Response};

use super::connection::Connection;
use super::pool;
use super::{PoolFsm, Client};


pub enum Fsm<R: Client> {
    Pool(Arc<Mutex<pool::Pool<R>>>),
    Client(Arc<Mutex<pool::Pool<R>>>, Arc<Mutex<Connection<R>>>),
}

impl<R: Client> Fsm<R> {
    pub fn pool(arc: Arc<Mutex<pool::Pool<R>>>) -> Fsm<R> {
        Fsm::Pool(arc)
    }
}


impl<R: Client> Machine for PoolFsm<R> {
    type Context = R::Context;
    type Seed = R;
    fn create(seed: Self::Seed, _scope: &mut Scope<Self::Context>)
        -> Result<Self, Box<Error>>
    {
        unimplemented!();
    }
    fn ready(self, _events: EventSet, _scope: &mut Scope<Self::Context>)
        -> Response<Self, Self::Seed>
    {
        unreachable!();
    }
    fn spawned(self, _scope: &mut Scope<Self::Context>)
        -> Response<Self, Self::Seed>
    {
        // nothing to do
        Response::ok(self)
    }
    fn timeout(self, _scope: &mut Scope<Self::Context>)
        -> Response<Self, Self::Seed>
    {
        unreachable!();
    }
    fn wakeup(self, _scope: &mut Scope<Self::Context>)
        -> Response<Self, Self::Seed>
    {
        let mut to_spawn = None;
        match self.0 {
            Fsm::Pool(ref arc) =>  {
                let mut pool = arc.lock().unwrap();
                match pool.insertion_queue.pop() {
                    Some(x) => {
                        to_spawn = Some(x);
                    }
                    None => {}
                }
            }
            Fsm::Client(pool, conn) => {
                unimplemented!();
            }
        }
        match to_spawn {
            Some(x) => Response::spawn(self, x),
            None => Response::ok(self),
        }
    }
}
