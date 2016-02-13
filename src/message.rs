use std::io::Write;
use std::ascii::AsciiExt;

use rotor_stream::Buf;

use Version;

quick_error! {
    #[derive(Debug)]
    pub enum HeaderError {
        DuplicateContentLength {
            description("Content-Length is added twice")
        }
        DuplicateTransferEncoding {
            description("Transfer-Encoding is added twice")
        }
        TransferEncodingAfterContentLength {
            description("Transfer encoding added when Content-Length is \
                already specified")
        }
        ContentLengthAfterTransferEncoding {
            description("Content-Length added after Transfer-Encoding")
        }
        CantDetermineBodySize {
            description("Neither Content-Length nor Transfer-Encoding \
                is present in the headers")
        }
        BodyLengthHeader {
            description("Content-Length and Transfer-Encoding must be set \
                using the specialized methods")
        }
    }
}

#[derive(Debug)]
pub enum MessageState {
    /// Nothing has been sent
    ResponseStart { version: Version, body: Body, close: bool },
    RequestStart,
    /// Status line is already in the buffer
    Headers { body: Body, chunked: bool, close: bool, request: bool,
              content_length: Option<u64> },
    ZeroBodyMessage,  // When response body is Denied
    IgnoredBody, // When response body is Ignored
    FixedSizeBody(u64),
    ChunkedBody,
    Done,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Body {
    Normal,
    Ignored,  // HEAD requests, 304 responses
    Denied,  // 101, 204 responses (100 too if it is used here)
}

/// Represents both request message and response message
///
/// Specific wrappers are exposed in `server` and `client` modules.
/// This type is private for the crate
pub struct Message<'a>(&'a mut Buf, MessageState);

impl MessageState {
    pub fn with<'x, I>(self, out_buf: &'x mut Buf) -> I
        where I: From<Message<'x>>
    {
        Message(out_buf, self).into()
    }
    pub fn is_started(&self) -> bool {
        !matches!(*self,
            MessageState::RequestStart |
            MessageState::ResponseStart { .. })
    }
}

impl<'a> Message<'a> {
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
    /// When status is 10x we don't assert yet
    pub fn response_status(&mut self, code: u16, reason: &str) {
        use self::Body::*;
        use self::MessageState::*;
        match self.1 {
            ResponseStart { version, mut body, close } => {
                // Note we don't expect code 100 and 102 here, but
                // we don't assert on that for now. The point is that
                // responses 100 and 102 are interim. 100 is generated by
                // rotor-http itself and 102 should probably too. Or we
                // will have a special method in request for it, because
                // request will contain another (real) response status here.
                //
                // TODO(tailhook) should we assert?
                //
                write!(self.0, "{} {} {}\r\n", version, code, reason).unwrap();
                if code == 101 || code == 204 {
                    body = Denied;
                } else if body == Normal && code == 304 {
                    body = Ignored;
                }
                self.1 = Headers { body: body, request: false,
                                   content_length: None,
                                   chunked: false, close: close };
            }
            ref state => {
                panic!("Called status() method on response in a state {:?}",
                       state)
            }
        }
    }
    /// Write request line
    ///
    /// This puts request line into a buffer immediately. If you don't
    /// continue with request it will be sent to the network shortly.
    ///
    /// # Panics
    ///
    /// When request line is already written. It's expected that your request
    /// handler state machine will never call the method twice.
    pub fn request_line(&mut self, method: &str, path: &str, version: Version)
    {
        use self::Body::*;
        use self::MessageState::*;
        match self.1 {
            RequestStart => {
                write!(self.0, "{} {} {}\r\n", method, path, version).unwrap();
                // It's common to allow request body for GET, is it so
                // expected for the HEAD too? Other methods?
                self.1 = Headers { body: Normal, request: true,
                                   content_length: None, chunked: false,
                                   close: false };
            }
            ref state => {
                panic!("Called status() method on request in state {:?}",
                       state)
            }
        }
    }

    fn write_header(&mut self, name: &str, value: &[u8]) {
        self.0.write_all(name.as_bytes()).unwrap();
        self.0.write_all(b": ").unwrap();
        self.0.write_all(value).unwrap();
        self.0.write_all(b"\r\n").unwrap();
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
        use self::MessageState::*;
        use self::HeaderError::*;
        if name.eq_ignore_ascii_case("Content-Length")
            || name.eq_ignore_ascii_case("Transfer-Encoding") {
            return Err(BodyLengthHeader)
        }
        match self.1 {
            Headers { .. } => {
                self.write_header(name, value);
                Ok(())
            }
            ref state => {
                panic!("Called add_header() method on a message in state {:?}",
                       state)
            }
        }
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
        -> Result<(), HeaderError> {
        use self::MessageState::*;
        use self::HeaderError::*;
        match self.1 {
            Headers { content_length: Some(_), .. } => {
                return Err(DuplicateContentLength);
            }
            Headers { chunked: true, .. } => {
                return Err(ContentLengthAfterTransferEncoding);
            }
            Headers { ref mut content_length, .. } => {
                *content_length = Some(n);
            }
            ref state => {
                panic!("Called add_length() method on message in state {:?}",
                       state)
            }
        }
        self.write_header("Content-Length", &n.to_string().into_bytes()[..]);
        Ok(())
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
        -> Result<(), HeaderError> {
            use self::MessageState::*;
            use self::HeaderError::*;
            match self.1 {
                Headers { content_length: Some(_), .. } => {
                    return Err(TransferEncodingAfterContentLength);
                }
                Headers { chunked: true, .. } => {
                    return Err(DuplicateTransferEncoding);
                }
                Headers { ref mut chunked, .. } => {
                    *chunked = true;
                }
            ref state => {
                panic!("Called add_chunked() method on message in state {:?}",
                       state)
            }
        }
        self.write_header("Transfer-Encoding", b"chunked");
        Ok(())
    }

    /// Returns true if at least `status()` method has been called
    ///
    /// This is mostly useful to find out whether we can build an error page
    /// or it's already too late.
    pub fn is_started(&self) -> bool {
        !matches!(self.1,
            MessageState::RequestStart |
            MessageState::ResponseStart { .. })
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
        use self::Body::*;
        use self::MessageState::*;
        if let Headers { close: true, .. } = self.1 {
            self.add_header("Connection", b"close").unwrap();
        }
        let result = match self.1 {
            Headers { body: Ignored, .. } => {
                self.1 = IgnoredBody;
                Ok(false)
            }
            Headers { body: Denied, .. } => {
                self.1 = ZeroBodyMessage;
                Ok(false)
            }
            Headers { body: Normal, content_length: Some(cl),
                      chunked: false, .. }
            => {
                self.1 = FixedSizeBody(cl);
                Ok(true)
            }
            Headers { body: Normal, content_length: None, chunked: true, .. }
            => {
                self.1 = ChunkedBody;
                Ok(true)
            }
            Headers { content_length: Some(_), chunked: true, .. }
            => unreachable!(),
            Headers { body: Normal, content_length: None, chunked: false,
                      request: true, .. }
            => {
                self.1 = ZeroBodyMessage;
                Ok(false)
            }
            Headers { body: Normal, content_length: None, chunked: false,
                      request: false,  .. }
            => Err(HeaderError::CantDetermineBodySize),
            ref state => {
                panic!("Called done_headers() method on  in a state {:?}",
                       state)
            }
        };
        self.0.write(b"\r\n").unwrap();
        result
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
        use self::MessageState::*;
        match self.1 {
            ZeroBodyMessage => {
                if data.len() != 0 {
                    panic!("Non-zero data length for the response where \
                            the response body is denied (101, 204)");
                }
            }
            FixedSizeBody(ref mut x) => {
                if data.len() as u64 > *x {
                    panic!("Fixed size response error. \
                        Bytes left {} but got additional {}", x, data.len());
                }
                self.0.write(data).unwrap();
                *x -= data.len() as u64;
            }
            ChunkedBody => {
                write!(self.0, "{:x}\r\n", data.len()).unwrap();
                self.0.write(data).unwrap();
            }
            ref state => {
                panic!("Called write_body() method on message \
                    in a state {:?}", state)
            }
        }
    }
    /// Returns true if `done()` method is already called and everything
    /// was okay.
    pub fn is_complete(&self) -> bool {
        matches!(self.1, MessageState::Done)
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
        use self::MessageState::*;
        match self.1 {
            ChunkedBody => {
                self.0.write(b"0\r\n").unwrap();
                self.1 = Done;
            }
            FixedSizeBody(0) => self.1 = Done,
            ZeroBodyMessage => self.1 = Done,
            IgnoredBody => self.1 = Done,
            Done => {}  // multiple invocations are okay
            ref state => {
                panic!("Called done() method on response in a state {:?}",
                       state);
            }
        }
    }

    pub fn state(self) -> MessageState {
        self.1
    }
    pub fn decompose(self) -> (&'a mut Buf, MessageState) {
        (self.0, self.1)
    }
}

#[cfg(test)]
mod test {
    use rotor_stream::Buf;
    use super::{Message, MessageState, Body};
    use Version;

    #[test]
    fn message_size() {
        // Just to keep track of size of structure
        assert_eq!(::std::mem::size_of::<MessageState>(), 24);
    }

    fn do_request<F: FnOnce(Message)>(fun: F) -> Buf {
        let mut buf = Buf::new();
        fun(MessageState::RequestStart.with(&mut buf));
        return buf;
    }
    fn do_response10<F: FnOnce(Message)>(fun: F) -> Buf {
        let mut buf = Buf::new();
        fun(MessageState::ResponseStart {
            version: Version::Http10,
            body: Body::Normal,
            close: false,
        }.with(&mut buf));
        return buf;
    }
    fn do_response11<F: FnOnce(Message)>(close: bool, fun: F) -> Buf {
        let mut buf = Buf::new();
        fun(MessageState::ResponseStart {
            version: Version::Http11,
            body: Body::Normal,
            close: close,
        }.with(&mut buf));
        return buf;
    }

    #[test]
    fn minimal_request() {
        assert_eq!(&do_request(|mut msg| {
            msg.request_line("GET", "/", Version::Http10);
            msg.done_headers().unwrap();
            msg.done();
        })[..], "GET / HTTP/1.0\r\n\r\n".as_bytes());
    }

    #[test]
    fn minimal_response() {
        assert_eq!(&do_response10(|mut msg| {
            msg.response_status(200, "OK");
            msg.add_length(0).unwrap();
            msg.done_headers().unwrap();
            msg.done();
        })[..], "HTTP/1.0 200 OK\r\nContent-Length: 0\r\n\r\n".as_bytes());
    }

    #[test]
    fn minimal_response11() {
        assert_eq!(&do_response11(false, |mut msg| {
            msg.response_status(200, "OK");
            msg.add_length(0).unwrap();
            msg.done_headers().unwrap();
            msg.done();
        })[..], "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".as_bytes());
    }

    #[test]
    fn close_response11() {
        assert_eq!(&do_response11(true, |mut msg| {
            msg.response_status(200, "OK");
            msg.add_length(0).unwrap();
            msg.done_headers().unwrap();
            msg.done();
        })[..], concat!("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n",
                        "Connection: close\r\n\r\n").as_bytes());
    }
}
