use std::time::Duration;

use rotor::{Scope, Time};

use recvmode::RecvMode;
use super::error::HttpError;
use super::request::Head;
use super::Response;


/// A handler of server-side HTTP
///
/// Used for all versions of HTTP
pub trait Server: Sized {
    type Context;
    /// Each request gets a clone of a Seed in `headers_received()` handler
    type Seed: Clone;
    /// Encountered when headers received.
    ///
    /// Returns self, mode and timeout for reading whole request.
    ///
    /// This handler decides whether a request is fully buffered or whether
    /// we need to read request body by chunk. It's recommended to return
    /// Buffered up to certain size, or at least for zero-length requests.
    ///
    /// You may start building a response right here, or wait for
    /// the next event.
    ///
    /// In case there is Expect header, the successful (non-None) return of
    /// this handler means we should return a `100 Expect` result. But if you
    /// have started building response right here, we skip expect header, as
    /// normal response code is good enough for browser. (Are there ugly
    /// proxies that propagate 100 Expect but does buffer response headers?)
    ///
    /// Note that `head` is passed here once, and forgotten by the
    /// protocol. If you need it later it's your responsibility to store it
    /// somewhere.
    fn headers_received(seed: Self::Seed, head: Head, response: &mut Response,
        scope: &mut Scope<Self::Context>)
        -> Option<(Self, RecvMode, Time)>;

    /// Called when full request is received in buffered mode.
    ///
    /// Note that even if you return None from handler, the data already
    /// written in Response is used and rotor-http does as much as it can
    /// to produce a valid response.
    fn request_received(self, data: &[u8], response: &mut Response,
        scope: &mut Scope<Self::Context>)
        -> Option<Self>;

    /// Called when request become invalid between `request_start()`
    /// and `request_received/request_end`
    ///
    /// You may put error page here if response is not `is_started()`. Or you
    /// can finish response in case you can send something meaningfull anyway.
    /// Otherwise, response will be filled with `BadRequest` from
    /// `emit_error_page` or connection closed immediately (if `is_started()`
    /// is true). You can't continue request processing after this handler is
    /// called.
    ///
    /// Currently it is called for two reasons:
    ///
    /// 1. Invalid chunked encoding
    /// 2. End of stream before number of bytes mentioned in Content-Length
    ///
    /// It's never called on a timeout.
    // TODO(tailhook) should there be some reason?
    fn bad_request(self, _response: &mut Response,
        _scope: &mut Scope<Self::Context>)
    {}

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
        scope: &mut Scope<Self::Context>)
        -> Option<Self>;

    /// End of request body, only for Progressive requests
    fn request_end(self, response: &mut Response,
        scope: &mut Scope<Self::Context>)
        -> Option<Self>;

    /// Request timeout occurred
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
    fn timeout(self, response: &mut Response, scope: &mut Scope<Self::Context>)
        -> Option<(Self, Time)>;
    fn wakeup(self, response: &mut Response, scope: &mut Scope<Self::Context>)
        -> Option<Self>;

    /// A bad request occured
    ///
    /// You should send a complete response in this handler.
    ///
    /// This is a static method, since it often called when there is no
    /// instance of request at all. You may override a page for specific
    /// request in `bad_request()` handler, if request headers are already
    /// parsed. If `bad_request()` emits anything this method is not called.
    ///
    /// You may also use (parts of) `seed` value or a context to determine
    /// the correct error page.
    ///
    /// You can also fallback to a default handler for pages you don't want
    /// to render.
    fn emit_error_page(code: &HttpError, response: &mut Response,
        _seed: &Self::Seed, _scope: &mut Scope<Self::Context>)
    {

        let (status, reason) = code.http_status();
        response.status(status, reason);
        let data = format!("<h1>{} {}</h1>\n\
            <p><small>Served for you by rotor-http</small></p>\n",
            status, reason);
        let bytes = data.as_bytes();
        response.add_length(bytes.len() as u64).unwrap();
        response.add_header("Content-Type", b"text/html").unwrap();
        response.done_headers().unwrap();
        response.write_body(bytes);
        response.done();
    }

    /// A timeout for idle keep-alive connection
    ///
    /// Default is 120 seconds
    fn idle_timeout(_seed: &Self::Seed, _scope: &mut Scope<Self::Context>)
        -> Duration
    {
        return Duration::new(120, 0);
    }
    /// A timeout for receiving at least one byte from headers
    ///
    /// Default is 45 seconds
    fn header_byte_timeout(_seed: &Self::Seed,
        _scope: &mut Scope<Self::Context>)
        -> Duration
    {
        return Duration::new(45, 0);
    }
    /// A timeout for sending full response body to the (slow) client
    ///
    /// Default is 3600 seconds (one hour)
    fn send_response_timeout(_seed: &Self::Seed,
        _scope: &mut Scope<Self::Context>)
        -> Duration
    {
        return Duration::new(3600, 0);
    }
}
