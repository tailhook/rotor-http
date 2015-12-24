use rotor_stream::{Transport, StreamSocket};

use super::{Response, ResponseImpl};


/// This response is returned when Response is dropping without writing
/// anything to the buffer. In any real scenario this page must never appear.
/// If it is, this probably means there is a bug somewhere. For example,
/// emit_error_page has returned without creating a real error response.
static NOT_IMPLEMENTED_HEAD: &'static str = concat!(
    "HTTP/1.0 501 Not Implemented\r\n",
    "Content-Type: text/plain\r\n",
    "Content-Length: 22\r\n",
    "\r\n",
    "501 Not Implemented\r\n",
    );
static NOT_IMPLEMENTED: &'static str = concat!(
    "HTTP/1.0 501 Not Implemented\r\n",
    "Content-Type: text/plain\r\n",
    "Content-Length: 22\r\n",
    "\r\n",
    );


impl<'a, 'b: 'a, S: StreamSocket> Response<'a, 'b, S> {
    pub fn new<'x, 'y>(trans: &'x mut Transport<'y, S>, is_head: bool)
        -> Response<'x, 'y, S>
    {
        use super::ResponseBody::*;
        Response(trans, ResponseImpl::Start {
            body: if is_head { Ignored } else { Normal },
        })
    }

    // TODO(tailhook) probably return true if response is okay, and it's
    // not a problem to reuse this keep-alive connection
    pub fn done(self) {
        use super::ResponseImpl::*;
        use super::ResponseBody::*;
        let Response(trans, me) = self;
        match me {
            Start { body: Denied } | Start { body: Ignored } => {
                trans.output().extend(NOT_IMPLEMENTED_HEAD.as_bytes());
            }
            Start { body: Normal } => {
                trans.output().extend(NOT_IMPLEMENTED.as_bytes());
            }
            Headers { body: Denied, .. } | Headers { body: Ignored, .. }=> {
                // Just okay
            }
            Headers { body: Normal, content_length: Some(0), chunked: false }
            => {
                // Just okay
            }
            Headers { body: Normal, content_length: Some(_), chunked: false }
            => {
                // TODO(tailhook) incomplete response, what to do?
                // should we close connection?
                unimplemented!();
            }
            Headers { body: Normal, content_length: _, chunked: true } => {
                // TODO(tailhook) emit headers and last chunk?
                unimplemented!();
            }
            Headers { body: Normal, content_length: None, chunked: false } => {
                // TODO(tailhook) probably incomplete headers, right?
                unimplemented!();
            }
            ZeroBodyResponse | FixedSizeBody(0) => {
                // Okay too
            }
            FixedSizeBody(usize) => {
                // Incomplete response but we have nothing to do
                // TODO(tailhook) log the error?
            }
            ChunkedBody => {
                // TODO(tailhook) emit last chunk?
                unimplemented!();
            }
        }
    }
}
