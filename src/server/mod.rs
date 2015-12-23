pub mod request;
pub mod protocol;
pub mod context;


enum ResponseImpl {
    Start,
    StatusLine,
    Headers { content_length: Option<usize>, chunked: bool },
    FixedSizeBody(usize),
    ChunkedBody,
}

pub struct Response(ResponseImpl);
