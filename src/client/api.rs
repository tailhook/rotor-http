use std::net::SocketAddr;

use super::{Pool, Client};

impl<R: Client> Pool<R> {
    /// Request to a specific host
    pub fn request_to(&self, addr: SocketAddr, req: R) {
        unimplemented!();
    }
}
