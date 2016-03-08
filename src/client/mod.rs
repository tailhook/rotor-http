//! HTTP Client implementation
//!
//! Currently we aim to provide only HTTP/1.x implementation. We want to
//! provide HTTP/2.0 and TLS implementation with exactly the same protocol.
//! But it's yet unproven if it is possible.
//!
//! Also DNS resolving is not implemented yet.
//!

use std::net::SocketAddr;

use rotor::{Scope, Response, Void};
use rotor::mio::tcp::TcpStream;
use rotor_stream;

mod request;
mod head;
mod protocol;
mod parser;
mod connection;

pub use version::Version;
pub use self::request::{Request};
pub use self::protocol::{Client, Requester, Task};
pub use self::head::Head;
pub use recvmode::RecvMode;

use self::parser::Parser;

// TODO(tailhook) MAX_HEADERS_SIZE can be moved to Context
// (i.e. made non-constant), but it's more of a problem for MAX_HEADERS_NUM
// because that would mean we can't allocate array of headers on the stack
// so performance will degrade. Customizing MAX_HEADERS_SIZE is not very
// useful on it's own

/// Note httparse requires we preallocate array of this size so be wise
pub const MAX_HEADERS_NUM: usize = 256;
/// This one is not preallocated, but too large buffer is of limited use
/// because of previous parameter.
pub const MAX_HEADERS_SIZE: usize = 16384;
/// Maximum length of chunk size line. it would be okay with 12 bytes, but in
/// theory there might be some extensions which we probably should skip
///
/// Note: we don't have a limit on chunk body size. In buffered request mode
/// it's limited by either memory or artificial limit returned from handler.
/// In unbuffered mode we can process chunk of unlimited size as long as
/// request handler is able to handle it.
pub const MAX_CHUNK_HEAD: usize = 128;

/// A state machine for ad-hoc requests
pub type Fsm<P, S> = rotor_stream::Stream<Parser<P, S>>;
/// A state machine for persistent connections
pub type Persistent<P, S> = rotor_stream::Persistent<Parser<P, S>>;

/// Structure that describes current connection state
///
/// In `Client::wakeup` you may check whether you can send a request using
/// `Connection::is_idle`
pub struct Connection {
    idle: bool,
}

pub fn connect_tcp<P: Client>(
    scope: &mut Scope<<P::Requester as Requester>::Context>,
    addr: &SocketAddr, seed: P::Seed)
    -> Response<Fsm<P, TcpStream>, Void>
{
    let sock = match TcpStream::connect(&addr) {
        Ok(sock) => sock,
        Err(e) => return Response::error(Box::new(e)),
    };
    rotor_stream::Stream::new(sock, seed, scope)
}
