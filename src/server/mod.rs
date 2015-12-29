mod request;
mod protocol;
mod context;
mod parser;
mod body;
mod response;

use netbuf::Buf;
use hyper::method::Method::Head;
use hyper::version::HttpVersion as Version;

use self::request::Head;
use self::response::{NOT_IMPLEMENTED_HEAD, NOT_IMPLEMENTED};

// TODO(tailhook) MAX_HEADERS_SIZE can be moved to Context
// (i.e. made non-constant), but it's more of a problem for MAX_HEADERS_NUM
// because that would mean we can't allocate array of headers on the stack
// so performance will degrade. Customizing MAX_HEADERS_SIZE is not very
// useful on it's own

/// Note httparse requires we preallocate array of this size so be wise
pub const MAX_HEADERS_NUM: usize = 256;
/// This one is not preallocated, but too large buffer is of limited use
/// because of previous parameter.
pub const MAX_HEADERS_SIZE: usize = 16384;
/// Maximum length of chunk size line. it would be okay with 12 bytes, but in
/// theory there might be some extensions which we probably should skip
///
/// Note: we don't have a limit on chunk body size. In buffered request mode
/// it's limited by either memory or artificial limit returned from handler.
/// In unbuffered mode we can process chunk of unlimited size as long as
/// request handler is able to handle it.
pub const MAX_CHUNK_HEAD: usize = 128;


#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ResponseBody {
    Normal,
    Ignored,  // HEAD requests, 304 responses
    Denied,  // 101, 204 responses (100 too if it is used here)
}


#[derive(Debug)]
enum ResponseImpl {
    /// Nothing has been sent
    Start { version: Version, body: ResponseBody },
    /// Status line is already in the buffer
    Headers { body: ResponseBody,
              content_length: Option<u64>, chunked: bool },
    ZeroBodyResponse,  // When response body is Denied
    IgnoredBody, // When response body is Ignored
    FixedSizeBody(u64),
    ChunkedBody,
    Done,
}

pub struct Response<'a>(&'a mut Buf, ResponseImpl);

impl ResponseImpl {
    fn with<'x>(self, out_buf: &'x mut Buf) -> Response<'x> {
        Response(out_buf, self)
    }
}

impl<'a> Response<'a> {
    fn internal(self) -> ResponseImpl {
        self.1
    }

    /// This is used for error pages, where it's impossible to parse input
    /// headers (i.e. get Head object needed for `Response::new`)
    fn simple<'x>(out_buf: &'x mut Buf, is_head: bool) -> Response<'x>
    {
        use self::ResponseBody::*;
        Response(out_buf, ResponseImpl::Start {
            body: if is_head { Ignored } else { Normal },
            // Always assume HTTP/1.0 when version is unknown
            version: Version::Http10,
        })
    }

    /// Creates new response by extracting needed fields from Head
    fn new<'x>(out_buf: &'x mut Buf, head: &Head) -> Response<'x>
    {
        use self::ResponseBody::*;
        // TODO(tailhook) implement Connection: Close,
        // (including explicit one in HTTP/1.0) and maybe others
        Response(out_buf, ResponseImpl::Start {
            body: if head.method == Head { Ignored } else { Normal },
            version: head.version,
        })
    }

    /// Returns true if it's okay too proceed with keep-alive connection
    fn finish(self) -> bool {
        use self::ResponseImpl::*;
        use self::ResponseBody::*;
        if self.is_complete() {
            return true;
        }
        let Response(buf, me) = self;
        match me {
            // If response is not even started yet, send something to make
            // debugging easier
            Start { body: Denied, .. } | Start { body: Ignored, .. } => {
                buf.extend(NOT_IMPLEMENTED_HEAD.as_bytes());
            }
            Start { body: Normal, .. } => {
                buf.extend(NOT_IMPLEMENTED.as_bytes());
            }
            _ => {}
        }
        return false;
    }
}
