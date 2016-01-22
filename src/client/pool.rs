use std::collections::HashMap;
use std::net::SocketAddr;
use rotor::{Notifier};

use super::{Client};

pub struct Connection<R> {
    pub notifier: Notifier,
    pub pending_request: Option<R>,
}

pub struct Chunk<R: Client> {
    pub idle: Vec<Connection<R>>,
    pub busy: Vec<Connection<R>>,
}

pub struct Pool<R: Client> {
    pub connections: HashMap<SocketAddr, Chunk<R>>,
    pub insertion_queue: Vec<(SocketAddr, R)>,
    pub notifier: Notifier,
}

impl<R: Client> Pool<R> {
    pub fn new(notifier: Notifier) -> Pool<R> {
        Pool {
            connections: HashMap::new(),
            insertion_queue: Vec::new(),
            notifier: notifier,
        }
    }
}

impl<R:Client> Chunk<R> {
    pub fn new() -> Chunk<R> {
        Chunk {
            idle: Vec::new(),
            busy: Vec::new(),
        }
    }
}
