use rotor::Scope;
use rotor_stream::{Deadline};

use recvmode::RecvMode;
use super::{Head, Request};


/// A handler of client-side HTTP
///
/// Used for all versions of HTTP.
///
/// Note: the interface allows you to receive the response before the whole
/// request is sent to the server. This is only occasionally useful (for
/// example if your server is some kind of encoding service that processes
/// data chunk by chunk). But if response header for some reason is received
/// before request is sent, the connection will be closed, as it probably
/// means that either server is misbehaving or we encounter some
/// out of sync behavior (which is enforced by rust type system so should
/// never happen, unless there is a bug in rotor-http), otherwise we may
/// have a cache poisoning security issue.
pub trait Client: Sized {
    type Context;

    fn byte_timeout(&self) -> Option<Self>;

    /// Populates a request
    ///
    /// This receives a `Request` object, which is a "builder". When you
    /// add things to it they are written directly to the buffer. This way
    /// you don't have to allocate temporary memory for map of headers in
    /// case you construct request programmatically.
    ///
    /// While you can continue sending request *body* when response headers
    /// are received. You must either send request *headers* in this handler
    /// or arrange this state machine to be waken up, because no actions will
    /// be invoked later unless response headers are sent.
    fn prepare_request(self, req: &mut Request) -> Option<Self>;

    /// Encountered when headers received
    ///
    /// Returns self, mode and timeout for reading whole response.
    ///
    /// This handler decides whether response is fully buffered or whether
    /// we need to read response body by chunk. It's recommended to return
    /// Buffered up to certain size, or at least for zero-length response.
    ///
    /// In case there is Expect header, the successful (non-None) return of
    /// this handler means we shoul return `100 Expect` result
    fn headers_received(self, head: &Head, request: &mut Request,
        scope: &mut Scope<Self::Context>)
        -> Option<(Self, RecvMode, Deadline)>;

    /// Called immediately after `headers_received`.
    ///
    /// Note that `head` is passed here once, and forgotten by the
    /// protocol. If you need it later it's your responsibility to store it
    /// somewhere.
    ///
    /// You may start building a response right here, or wait for
    /// the next event.
    fn response_start(self, head: Head, request: &mut Request,
        scope: &mut Scope<Self::Context>)
        -> Option<Self>;

    /// Called when full response is received in buffered mode
    ///
    /// Note: you can't continue with connection here. But you can finish
    /// the request (although, it's probably doesn't make too much sense)
    fn response_received(self, data: &[u8], request: &mut Request,
        scope: &mut Scope<Self::Context>);

    /// Called when response become invalid between `prepare_request()`
    /// and `response_received/response_end`
    ///
    /// This is useful mostly to notify the requestor that it will not have
    /// anything. Note this event doesnt' relate to any HTTP status codes.
    /// They are treated as normal responses by the state machine.
    ///
    /// Currently it is called for two reasons:
    ///
    /// 1. Invalid chunked encoding
    /// 2. End of stream before number of bytes mentioned in Content-Length
    /// 3. Trying to send response body with 204 status code
    ///
    /// It's never called on a timeout.
    // TODO(tailhook) should there be some reason?
    fn bad_response(self, _scope: &mut Scope<Self::Context>)
    {}

    /// Received chunk of data
    ///
    /// Whey you return `Progressive(nbytes)` from headers received, you
    /// may expect than chunk will be at least of `nbytes` of length. But
    /// you must not rely on that for few reasons:
    ///
    /// 1. Last chunk of response body may be smaller
    /// 2. Chunk is read up to some buffer size, which is heuristically
    ///    determined, and is usually larger than `nbytes`
    /// 3. Currently for chunked encoding we don't merge chunks, so last
    ///    part of each chunk may be shorter as `nbytes`
    fn response_chunk(self, chunk: &[u8], request: &mut Request,
        scope: &mut Scope<Self::Context>)
        -> Option<Self>;

    /// End of response body, only for Progressive responses
    ///
    /// Note: you can't continue with connection here. But you can finish
    /// the request (although, it's probably doesn't make too much sense)
    fn response_end(self, request: &mut Request,
        scope: &mut Scope<Self::Context>);

    /// Request timeout occured
    ///
    /// Unless you've returned the new timeout connection will be closed after
    /// the event.
    fn timeout(self, request: &mut Request, scope: &mut Scope<Self::Context>)
        -> Option<(Self, Deadline)>;
    fn wakeup(self, request: &mut Request, scope: &mut Scope<Self::Context>)
        -> Option<Self>;
}
