pub mod request;
pub mod protocol;
pub mod context;
pub mod parser;


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


enum ResponseImpl {
    Start,
    StatusLine,
    Headers { content_length: Option<usize>, chunked: bool },
    FixedSizeBody(usize),
    ChunkedBody,
}

pub struct Response(ResponseImpl);
