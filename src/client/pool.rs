use std::collections::HashMap;

use ip::IpAddr;
use rotor::Port;

use super::{RequestCreator};

pub struct Pool<R: RequestCreator> {
    idle: HashMap<IpAddr, Vec<Port<R>>>,
}

impl<R: RequestCreator> Pool<R> {
    pub fn new() -> Pool<R> {
        Pool {
            idle: HashMap::new(),
        }
    }
}
