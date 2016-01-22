use std::sync::{Arc, Mutex};
use std::net::SocketAddr;
use std::error::Error;

use rotor::{Machine, Scope, EventSet, PollOpt, Response};
use rotor::mio::tcp::TcpStream;

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
    type Seed = (SocketAddr, Arc<Mutex<pool::Pool<R>>>, R);
    fn create((addr, pool, cli): Self::Seed, scope: &mut Scope<Self::Context>)
        -> Result<Self, Box<Error>>
    {
        let conn = try!(TcpStream::connect(&addr));
        try!(scope.register(&conn, EventSet::writable(), PollOpt::edge()));
        Ok(PoolFsm(
            Fsm::Client(pool, Arc::new(Mutex::new(Connection::new(cli, conn))))))
    }
    fn ready(self, _events: EventSet, _scope: &mut Scope<Self::Context>)
        -> Response<Self, Self::Seed>
    {
        match self.0 {
            Fsm::Pool(ref arc) => unreachable!(),
            Fsm::Client(pool, conn) => {
                unimplemented!();
            }
        }
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
                    Some((addr, req)) => {
                        to_spawn = Some((addr, arc.clone(), req));
                    }
                    None => {}
                }
            }
            Fsm::Client(pool, conn) => {
                unimplemented!();
            }
        }
        match to_spawn {
            Some(seed) => Response::spawn(self, seed),
            None => Response::ok(self),
        }
    }
}
