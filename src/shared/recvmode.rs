/// This type is returned from `headers_received` handler of either
/// client client or server protocol handler
///
/// The marker is used to denote whether you want to have the whole request
/// buffered for you or read chunk by chunk.
///
/// The `Progressive` (chunk by chunk) mode is mostly useful for proxy servers.
/// Or it may be useful if your handler is able to parse data without holding
/// everything in the memory.
///
/// Otherwise, it's best to use `Buffered` mode (for example, comparing with
/// using your own buffering). We do our best to optimize it for you.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecvMode {
    /// Download whole message body (request or response) into the memory.
    ///
    /// The argument is maximum size of the body. The Buffered variant
    /// works equally well for Chunked encoding and for read-util-end-of-stream
    /// mode of HTTP/1.0, so sometimes you can't know the size of the request
    /// in advance. Note this is just an upper limit it's neither buffer size
    /// nor *minimum* size of the body.
    ///
    /// Note the buffer size is asserted on if it's bigger than max buffer size
    Buffered(usize),
    /// Fetch data chunk-by-chunk.
    ///
    /// The parameter denotes minimum number of bytes that may be passed
    /// to the protocol handler. This is for performance tuning (i.e. less
    /// wake-ups of protocol parser). But it's not an input buffer size. The
    /// use of `Progressive(1)` is perfectly okay (for example if you use http
    /// request body as a persistent connection for sending multiple messages
    /// on-demand)
    Progressive(usize),
}
