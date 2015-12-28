use hyper::version::HttpVersion as Version;
use hyper::status::StatusCode;
use time::Duration;
use rotor::Scope;
use rotor_stream::{Deadline, StreamSocket};

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
pub trait Server<C: Context, S: StreamSocket>: Sized {
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
    fn request_start(self, head: Head, response: &mut Response<S>,
        scope: &mut Scope<C>)
        -> Option<Self>;

    /// Called when full request is received in buffered mode
    ///
    /// Note that even if you return None from handler, the data already
    /// written in Response is used and rotor-http does as much as it can
    /// to produce a valid response.
    fn request_received(self, data: &[u8], response: &mut Response<S>,
        scope: &mut Scope<C>)
        -> Option<Self>;


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
    fn request_chunk(self, chunk: &[u8], response: &mut Response<S>,
        scope: &mut Scope<C>)
        -> Option<Self>;

    /// End of request body, only for Progressive requests
    fn request_end(self, response: &mut Response<S>, scope: &mut Scope<C>)
        -> Option<Self>;

    fn timeout(self, response: &mut Response<S>, scope: &mut Scope<C>)
        -> Option<Self>;
    fn wakeup(self, response: &mut Response<S>, scope: &mut Scope<C>)
        -> Option<Self>;
}
