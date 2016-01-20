use std::collections::HashMap;

use ip::IpAddr;
use rotor::{Notifier};

use super::{RequestCreator};

pub struct Connection<R> {
    notifier: Notifier,
    pending_request: Option<R>,
}

pub struct Chunk<R: RequestCreator> {
    idle: Vec<Connection<R>>,
    busy: Vec<Connection<R>>,
}

pub struct Pool<R: RequestCreator> {
    connections: HashMap<IpAddr, Chunk<R>>,
    notifier: Notifier,
}

impl<R: RequestCreator> Pool<R> {
    pub fn new(notifier: Notifier) -> Pool<R> {
        Pool {
            connections: HashMap::new(),
            notifier: notifier,
        }
    }
}
