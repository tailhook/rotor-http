use std::net::SocketAddr;

use super::{Pool, Client};
use super::pool::Chunk;


impl<R: Client> Pool<R> {
    /// Request to a specific host
    pub fn request_to(&self, addr: SocketAddr, req: R) {
        let ref mut pool = &mut *self.0.lock().unwrap();
        let chunk = pool.connections.entry(addr).or_insert_with(Chunk::new);
        match chunk.idle.pop() {
            Some(x) => {
                unimplemented!();
            }
            None => {
                pool.insertion_queue.push(req);
                pool.notifier.wakeup().unwrap();
            }
        }
    }
}
