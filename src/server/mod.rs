mod request;
mod protocol;
mod context;
mod parser;
mod body;
mod response;

use rotor_stream::{Transport, StreamSocket};

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

pub enum ResponseBody {
    Normal,
    Ignored,  // HEAD requests, 304 responses
    Denied,  // 101, 204 responses (100 too if it is used here)
}


enum ResponseImpl {
    /// Nothing has been sent
    Start { body: ResponseBody },
    /// Status line is already in the buffer
    Headers { body: ResponseBody,
              content_length: Option<usize>, chunked: bool },
    ZeroBodyResponse,  // When allow_body = false
    FixedSizeBody(usize),
    ChunkedBody,
}

pub struct Response<'a, 'b: 'a, S>(&'a mut Transport<'b, S>, ResponseImpl)
    where S: StreamSocket;
