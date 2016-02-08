use hyper::status::StatusCode::{self, BadRequest};
use hyper::header::{ContentLength, TransferEncoding, Encoding};

use super::request::Head;


#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BodyKind {
    Fixed(u64),
    Upgrade,
    Chunked,
}

impl BodyKind {
    /// Implements the body length algorithm for requests:
    /// http://httpwg.github.io/specs/rfc7230.html#message.body.length
    ///
    /// The length of a request body is determined by one of the following
    /// (in order of precedence):
    ///
    /// 1. If the request contains a valid `Transfer-Encoding` header
    ///    with `chunked` as the last encoding the request is chunked
    ///    (3rd option in RFC).
    /// 2. If the request contains a valid `Content-Length` header
    ///    the request has the given length in octets
    ///    (5th option in RFC).
    /// 3. If neither `Transfer-Encoding` nor `Content-Length` are
    ///    present the request has an empty body
    ///    (6th option in RFC).
    /// 4. In all other cases the request is a bad request.
    pub fn parse(head: &Head) -> Result<BodyKind, StatusCode> {
        use self::BodyKind::*;
        if head.headers.has::<TransferEncoding>() {
            if let Some(items) = head.headers.get::<TransferEncoding>() {
                if items.last() == Some(&Encoding::Chunked) {
                    return Ok(Chunked);
                }
            }
        } else if head.headers.has::<ContentLength>() {
            if let Some(&ContentLength(x)) = head.headers.get::<ContentLength>() {
               return Ok(Fixed(x));
           }
       } else {
           return Ok(Fixed(0));
       }
       return Err(BadRequest);
    }
}
