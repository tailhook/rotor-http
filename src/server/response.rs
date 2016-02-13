use rotor_stream::Buf;

use message::{MessageState, Message, HeaderError};
use Version;


/// This response is returned when Response is dropping without writing
/// anything to the buffer. In any real scenario this page must never appear.
/// If it is, this probably means there is a bug somewhere. For example,
/// emit_error_page has returned without creating a real error response.
pub const NOT_IMPLEMENTED: &'static str = concat!(
    "HTTP/1.0 501 Not Implemented\r\n",
    "Content-Type: text/plain\r\n",
    "Content-Length: 21\r\n",
    "\r\n",
    "501 Not Implemented\r\n",
    );
pub const NOT_IMPLEMENTED_HEAD: &'static str = concat!(
    "HTTP/1.0 501 Not Implemented\r\n",
    "Content-Type: text/plain\r\n",
    "Content-Length: 21\r\n",
    "\r\n",
    );

pub struct Response<'a>(Message<'a>);

impl<'a> From<Message<'a>> for Response<'a> {
    fn from(msg: Message) -> Response {
        Response(msg)
    }
}

impl<'a> Response<'a> {
    /// Creates new response message by extracting needed fields from Head
    pub fn new(out_buf: &mut Buf, version: Version,
        is_head: bool, do_close: bool) -> Response
    {
        use message::Body::*;
        // TODO(tailhook) implement Connection: Close,
        // (including explicit one in HTTP/1.0) and maybe others
        MessageState::ResponseStart {
            body: if is_head { Ignored } else { Normal },
            version: version,
            close: do_close || version == Version::Http10,
        }.with(out_buf)
    }
    /// Returns true if it's okay too proceed with keep-alive connection
    pub fn finish(self) -> bool {
        use message::MessageState::*;
        use message::Body::*;
        if self.is_complete() {
            return true;
        }
        let (buf, me) = self.0.decompose();
        match me {
            // If response is not even started yet, send something to make
            // debugging easier
            ResponseStart { body: Denied, .. }
            | ResponseStart { body: Ignored, .. }
            => {
                buf.extend(NOT_IMPLEMENTED_HEAD.as_bytes());
            }
            ResponseStart { body: Normal, .. } => {
                buf.extend(NOT_IMPLEMENTED.as_bytes());
            }
            _ => {}
        }
        false
    }

    /// Write status line
    ///
    /// This puts status line into a buffer immediately. If you don't
    /// continue with request it will be sent to the network shortly.
    ///
    /// # Panics
    ///
    /// When status line is already written. It's expected that your request
    /// handler state machine will never call the method twice.
    ///
    /// When status is 100x
    pub fn status(&mut self, code: u16, reason: &str) {
        self.0.response_status(code, reason)
    }
    /// Add header to message
    ///
    /// Header is written into the output buffer immediately. And is sent
    /// as soon as the next loop iteration
    ///
    /// `Content-Length` header must be send using the `add_length` method
    /// and `Transfer-Encoding: chunked` must be set with the `add_chunked`
    /// method. These two headers are important for the security of HTTP.
    ///
    /// Note that there is currently no way to use a transfer encoding other
    /// than chunked.
    ///
    /// We return Result here to make implementing proxies easier. In the
    /// application handler it's okay to unwrap the result and to get
    /// a meaningful panic (that is basically an assertion).
    ///
    /// # Panics
    ///
    /// Panics when `add_header` is called in the wrong state.
    pub fn add_header(&mut self, name: &str, value: &[u8])
        -> Result<(), HeaderError>
    {
        self.0.add_header(name, value)
    }
    /// Add a content length to the message.
    ///
    /// The `Content-Length` header is written to the output buffer immediately.
    /// It is checked that there are no other body length headers present in the
    /// message. When the body is send the length is validated.
    ///
    /// # Panics
    ///
    /// Panics when `add_length` is called in the wrong state.
    pub fn add_length(&mut self, n: u64)
        -> Result<(), HeaderError>
    {
        self.0.add_length(n)
    }
    /// Sets the transfer encoding to chunked.
    ///
    /// Writes `Transfer-Encoding: chunked` to the output buffer immediately.
    /// It is assured that there is only one body length header is present
    /// and the body is written in chunked encoding.
    ///
    /// # Panics
    ///
    /// Panics when `add_chunked` is called in the wrong state.
    pub fn add_chunked(&mut self)
        -> Result<(), HeaderError>
    {
        self.0.add_chunked()
    }
    /// Returns true if at least `status()` method has been called
    ///
    /// This is mostly useful to find out whether we can build an error page
    /// or it's already too late.
    pub fn is_started(&self) -> bool {
        self.0.is_started()
    }
    /// Checks the validity of headers. And returns `true` if entity
    /// body is expected.
    ///
    /// Specifically `false` is returned when status is 101, 204, 304 or the
    /// request is HEAD. Which means in both cases where response body is
    /// either ignored (304, HEAD) or is denied by specification. But not
    /// when response is zero-length.
    ///
    /// Similarly to `add_header()` it's fine to `unwrap()` here, unless you're
    /// doing some proxying.
    ///
    /// # Panics
    ///
    /// Panics when response is in a wrong state
    pub fn done_headers(&mut self) -> Result<bool, HeaderError> {
        self.0.done_headers()
    }
    /// Write a chunk of the body
    ///
    /// Works both for fixed-size body and chunked body.
    ///
    /// For the chunked body each chunk is put into the buffer immediately
    /// prefixed by chunk size.
    ///
    /// For both modes chunk is put into the buffer, but is only sent when
    /// rotor-stream state machine is reached. So you may put multiple chunks
    /// into the buffer quite efficiently.
    ///
    /// For Ignored body you can `write_body` any number of times, it's just
    /// ignored. But it's more efficient to check it with `needs_body()`
    ///
    /// # Panics
    ///
    /// When response is in wrong state. Or there is no headers which
    /// determine response body length (either Content-Length or
    /// Transfer-Encoding)
    pub fn write_body(&mut self, data: &[u8]) {
        self.0.write_body(data)
    }
    /// Returns true if `done()` method is already called and everything
    /// was okay.
    pub fn is_complete(&self) -> bool {
        self.0.is_complete()
    }
    /// Writes needed final finalization data into the buffer and asserts
    /// that response is in the appropriate state for that.
    ///
    /// The method may be called multiple times
    ///
    /// # Panics
    ///
    /// When the response is in the wrong state or when Content-Length bytes
    /// are not written yet
    pub fn done(&mut self) {
        self.0.done()
    }
}

pub fn state(resp: Response) -> MessageState {
    resp.0.state()
}
