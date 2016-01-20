use std::collections::HashMap;
use std::net::SocketAddr;
use rotor::{Notifier};

use super::{Client};

pub struct Connection<R> {
    notifier: Notifier,
    pending_request: Option<R>,
}

pub struct Chunk<R: Client> {
    idle: Vec<Connection<R>>,
    busy: Vec<Connection<R>>,
}

pub struct Pool<R: Client> {
    connections: HashMap<SocketAddr, Chunk<R>>,
    notifier: Notifier,
}

impl<R: Client> Pool<R> {
    pub fn new(notifier: Notifier) -> Pool<R> {
        Pool {
            connections: HashMap::new(),
            notifier: notifier,
        }
    }
}
