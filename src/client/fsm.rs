use std::error::Error;

use void::{Void, unreachable};
use rotor::{Machine, Scope, EventSet, Response};

use super::{PoolFsm, RequestCreator};


impl<C, R: RequestCreator> Machine for PoolFsm<R, C> {
    type Context = C;
    type Seed = Void;
    fn create(seed: Self::Seed, _scope: &mut Scope<Self::Context>)
        -> Result<Self, Box<Error>>
    {
        unreachable(seed);
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
        unimplemented!();
    }
}
