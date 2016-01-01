use hyper::status::StatusCode;
use rotor::Scope;
use rotor_stream::{Deadline};

use super::context::Context;
use super::request::Head;
use super::Response;


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecvMode {
    /// Download whole request body into the memory.
    ///
    /// The argument is maximum size of the request. The Buffered variant
    /// works equally well for Chunked encoding and for read-util-end-of-stream
    /// mode of HTTP/1.0, so sometimes you can't know the size of the request
    /// in advance. Note this is just an upper limit it's neither buffer size
    /// nor minimum size of the body.
    ///
    /// Note the buffer size is asserted on if it's bigger than max buffer size
    Buffered(usize),
    /// Fetch data chunk-by-chunk
    ///
    /// The parameter denotes minimum number of bytes that may be passed
    /// to the protocol handler. This is for performance tuning (i.e. less
    /// wake-ups of protocol parser). But it's not an input buffer size. The
    /// use of `Progressive(1)` is perfectly okay (for example if you use http
    /// request body as a persistent connection for sending multiple messages
    /// on-demand)
    Progressive(usize),
}



/// A handler of server-side HTTP
///
/// Used for all versions of HTTP
pub trait Server<C: Context>: Sized {
    /// Encountered when headers received
    ///
    /// Returns self, mode and timeout for reading whole request.
    ///
    /// This handler decides whether request is fully buffered or whether
    /// we need to read request body by chunk. It's recommended to return
    /// Buffered up to certain size, or at least for zero-length request.
    ///
    /// In case there is Expect header, the successful (non-None) return of
    /// this handler means we shoul return `100 Expect` result
    fn headers_received(head: &Head, scope: &mut Scope<C>)
        -> Result<(Self, RecvMode, Deadline), StatusCode>;

    /// Called immediately after `headers_received`.
    ///
    /// Note that `head` is passed here once, and forgotten by the
    /// protocol. If you need it later it's your responsibility to store it
    /// somewhere.
    ///
    /// You may start building a response right here, or wait for
    /// the next event.
    fn request_start(self, head: Head, response: &mut Response,
        scope: &mut Scope<C>)
        -> Option<Self>;

    /// Called when full request is received in buffered mode
    ///
    /// Note that even if you return None from handler, the data already
    /// written in Response is used and rotor-http does as much as it can
    /// to produce a valid response.
    fn request_received(self, data: &[u8], response: &mut Response,
        scope: &mut Scope<C>)
        -> Option<Self>;

    /// Called when request become invalid between `request_start()`
    /// and `request_received/request_end`
    ///
    /// You may put error page here if response is not `is_started()`. Or you
    /// can finish response in case you can send something meaningfull anyway.
    /// Otherwise, response will be filled with `BadRequest` from `Context` or
    /// connection closed immediately (if `is_started()` is true). You
    /// can't continue request processing after this handler is called.
    ///
    /// Currently it is called for two reasons:
    ///
    /// 1. Invalid chunked encoding
    /// 2. End of stream before number of bytes mentioned in Content-Length
    ///
    /// It's never called on a timeout.
    // TODO(tailhook) should there be some reason?
    fn bad_request(self, _response: &mut Response, _scope: &mut Scope<C>) {}

    /// Received chunk of data
    ///
    /// Whey you return `Progressive(nbytes)` from headers received, you
    /// may expect than chunk will be at least of `nbytes` of length. But
    /// you must not rely on that for few reasons:
    ///
    /// 1. Last chunk of request body may be smaller
    /// 2. Chunk is read up to some buffer size, which is heuristically
    ///    determined, and is usually larger than `nbytes`
    /// 3. Currently for chunked encoding we don't merge chunks, so last
    ///    part of each chunk may be shorter as `nbytes`
    fn request_chunk(self, chunk: &[u8], response: &mut Response,
        scope: &mut Scope<C>)
        -> Option<Self>;

    /// End of request body, only for Progressive requests
    fn request_end(self, response: &mut Response, scope: &mut Scope<C>)
        -> Option<Self>;

    /// Request timeout occured
    ///
    /// This is only called if headers are already received but state machine
    /// is not yet finished. It drops down in two cases:
    ///
    /// 1. Receiving request
    /// 2. Sending response
    ///
    /// If you received timeout you can return the new one, send error, or
    /// finish response whatever you like. If response is not started yet at
    /// the time method returns, client gets 408 error code.
    ///
    /// Unless you've returned the new timeout connection will be closed after
    /// the event.
    fn timeout(self, response: &mut Response, scope: &mut Scope<C>)
        -> Option<(Self, Deadline)>;
    fn wakeup(self, response: &mut Response, scope: &mut Scope<C>)
        -> Option<Self>;
}
