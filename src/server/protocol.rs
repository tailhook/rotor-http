use hyper::version::HttpVersion as Version;
use hyper::status::StatusCode;
use time::Duration;
use rotor::Scope;
use rotor_stream::Deadline;

use super::context::Context;
use super::request::Head;
use super::Response;


pub enum RecvMode {
    /// Download whole request body into the memory
    Buffered,
    /// Fetch data chunk-by-chunk
    ///
    /// The parameter denotes minimum number of bytes that may be passed
    /// to the protocol handler. This is for performance tuning (i.e. less
    /// wake-ups of protocol parser). But'it's not read buffer size. The use
    /// of `Progressive(1)` is perfectly okay (for example if you use http
    /// request body as a persistent connection for sending multiple
    /// messages on-demand)
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
    /// Default implementation returns Buffered for all requests and allows
    /// to receive any request size. It's okay for quick prototypes but bad
    /// for the real work. (should we change that?)
    ///
    /// In case there is Expect header, the successful (non-None) return of
    /// this handler means we shoul return `100 Expect` result
    ///
    fn headers_received(self, head: &Head, scope: &mut Scope<C>)
        -> Result<(Self, RecvMode, Deadline), StatusCode>
    {
        Ok((self, RecvMode::Buffered,
            Deadline::now() + Duration::seconds(90)))
    }

    /// Called when full request is received in buffered mode
    ///
    /// Note that `head` is passed here once, and forgotten by the
    /// protocol. If you need it later it's your responsibility to store it
    /// somewhere.
    ///
    /// Note that even if you return None from handler, the data already
    /// written in Response is used and rotor-http does as much as it can
    /// to produce a valid response.
    fn request_received(self, head: Head, response: &mut Response,
        scope: &mut Scope<C>)
        -> Option<Self>;

    /// Called immediately after `headers_received`, if it returns
    /// `Progressive(_)`. You may need to store the head somewhere for later
    /// use. You may start building a response right here, or wait for
    /// next event.
    fn request_start(self, head: Head, response: &mut Response,
        scope: &mut Scope<C>)
        -> Option<Self>;

    /// Received chunk of data
    ///
    /// Whey you return `Progressive(nbytes)` from headers received, you
    /// may expect than chunk will be at least of `nbytes` of length. But
    /// you must not rely on that for two reasons:
    ///
    /// 1. Last chunk of request body may be smaller
    /// 2. Chunk is read up to some buffer size, which is heuristically
    ///    determined, and is usually larger than `nbytes`
    fn request_chunk(self, chunk: &[u8], response: &mut Response,
        scope: &mut Scope<C>)
        -> Option<Self>;

    /// End of request body, only for Progressive requests
    fn request_end(self, response: &mut Response, scope: &mut Scope<C>)
        -> Option<Self>;

    fn timeout(self, response: &mut Response, scope: &mut Scope<C>)
        -> Option<Self>;
    fn wakeup(self, response: &mut Response, scope: &mut Scope<C>)
        -> Option<Self>;
}
