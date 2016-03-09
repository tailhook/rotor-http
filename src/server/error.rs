use std::error::Error;
use std::str::Utf8Error;
use std::num::ParseIntError;

use httparse;


quick_error!{
    /// Error type which is passed to bad_request and emit_error_page
    ///
    /// Note, you should not match the enum values and/or make an exhaustive
    /// match over the enum. More errors will be added at will.
    ///
    /// Use HttpError trait instead
    #[derive(Debug)]
    pub enum RequestError {
        HeadersAreTooLarge {
            description("headers are larger than \
                         http::request::MAX_HEADERS_SIZE")
        }
        BadHeaders(e: httparse::Error) {
            from()
            description("error parsing headers")
            display(me) -> ("{}: {:?}", me.description(), e)
        }
        InvalidChunkSize(e: httparse::InvalidChunkSize) {
            from()
            description("error parsing chunk size")
        }
        DuplicateContentLength {
            description("duplicate `Content-Length` header in request")
        }
        HeadersReceived {
            description("request aborted in `headers_received()` handler")
        }
        PayloadTooLarge {
            description("payload is larger than is allowed by server settings")
        }
        PrematureEndOfStream {
            description("premature end of stream")
        }
        HeadersTimeout {
            description("timeout reading request headers")
        }
        RequestTimeout {
            description("timeout reading request body")
        }
        HandlerTimeout {
            description("timeout happened waiting for handler")
        }
        BadUtf8(err: Utf8Error) {
            from()
            description("bad utf8 in one of the crucial headers")
            display(me) -> ("{}: {}", me.description(), err)
        }
        BadContentLength(err: ParseIntError) {
            description("error parsing `Content-Length` header")
            display(me) -> ("{}: {}", me.description(), err)
        }
    }
}

/// A trait which represents an error which can be formatted as HTTP error page
pub trait HttpError {
    /// Return HTTP status code and status text
    ///
    /// The status text and code are also printed on the error page itself
    fn http_status(&self) -> (u16, &'static str);
}

impl HttpError for RequestError {
    fn http_status(&self) -> (u16, &'static str) {
        use self::RequestError::*;
        match *self {
            HeadersAreTooLarge => (431, "Request Header Fields Too Large"),
            BadHeaders(_) => (400, "Bad Request"),
            BadUtf8(_) => (400, "Bad Request"),
            BadContentLength(_) => (400, "Bad Request"),
            InvalidChunkSize(_) => (400, "Bad Request"),
            DuplicateContentLength => (400, "Bad Request"),
            HeadersReceived => (400, "Bad Request"),
            PayloadTooLarge => (413, "Payload Too Large"),
            HeadersTimeout => (408, "Request Timeout"),
            RequestTimeout => (408, "Request Timeout"),
            HandlerTimeout => (504, "Gateway Timeout"),
            // This one almost never reaches the destination
            PrematureEndOfStream => (400, "Bad Request"),
        }
    }
}
