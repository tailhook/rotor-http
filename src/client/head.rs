use hyper::version::HttpVersion as Version;
use hyper::method::Method;
use hyper::status::StatusCode;
use hyper::header::Headers;
use httparse;


#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BodyKind {
    Fixed(u64),
    Chunked,
    Eof,
}

pub struct Head<'a> {
    pub version: Version,
    pub code: u16,
    pub reason: &'a str,
    pub headers: &'a [httparse::Header<'a>],
    pub body_kind: BodyKind,
    pub close: bool,
}

        /*
        /// Implements the body length algorithm for requests:
        /// http://httpwg.github.io/specs/rfc7230.html#message.body.length
        use self::BodyKind::*;
        if head.request_method == Method::Head ||
           head.status.class() == Informational ||
           head.status == NoContent ||
           head.status == NotModified
        {
           Ok(Fixed(0))
        } else if let Some(items) = head.headers.get::<TransferEncoding>() {
            // TODO(tailhook) add Connection: close to headers if headers
            // have Content-Length too
            if items.last() == Some(&Encoding::Chunked) {
                Ok(Chunked)
            } else {
                Err(())
            }
        } else if let Some(&ContentLen(x)) = head.headers.get::<ContentLen>() {
            Ok(Fixed(x))
        } else {
            Ok(Eof)
        }
        */
