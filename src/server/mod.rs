//! HTTP Server implementation
//!
//! Currently there is only HTTP/1.x implementation. We want to provide
//! HTTP/2.0 and HTTPS
//!
use rotor::mio::TryAccept;
pub use rotor_stream::{Accept, Stream};

pub use recvmode::RecvMode;
pub use version::Version;
pub use self::body::BodyKind;
pub use self::context::Context;
pub use self::parser::Parser;
pub use self::protocol::Server;
pub use self::request::Head;
pub use self::response::Response;

mod body;
mod context;
mod parser;
mod protocol;
mod request;
mod response;


// TODO(tailhook) MAX_HEADERS_SIZE can be moved to Context
// (i.e. made non-constant), but it's more of a problem for MAX_HEADERS_NUM
// because that would mean we can't allocate array of headers on the stack
// so performance will degrade. Customizing MAX_HEADERS_SIZE is not very
// useful on it's own

/// The maximum allowed number of headers in a request.
///
/// Note httparse requires we preallocate array of this size so be wise.
pub const MAX_HEADERS_NUM: usize = 256;

/// The maximum size of request headers in bytes.
///
/// This one is not preallocated, but too large buffer is of limited use
/// because of `MAX_HEADERS_NUM` parameter.
pub const MAX_HEADERS_SIZE: usize = 16384;

/// The maximum length of chunk size line in bytes.
///
/// It would be okay with 12 bytes, but in theory there might be some extensions
/// which we skip. Note that the header is also used for padding.
///
/// Note: we don't have a limit on chunk body size. In buffered request mode
/// it's limited by either memory or artificial limit returned from handler.
/// In unbuffered mode we can process chunk of unlimited size as long as
/// request handler is able to handle it.
pub const MAX_CHUNK_HEAD: usize = 128;

/// Shortcut type for server state machines.
pub type Fsm<M, L> = Accept<Stream<Parser<M, <L as TryAccept>::Output>>, L>;
