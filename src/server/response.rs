use rotor_stream::{Transport, StreamSocket};

use super::{Response, ResponseImpl};


/// This response is returned when Response is dropping without writing
/// anything to the buffer. In any real scenario this page must never appear.
/// If it is, this probably means there is a bug somewhere. For example,
/// emit_error_page has returned without creating a real error response.
pub const NOT_IMPLEMENTED_HEAD: &'static str = concat!(
    "HTTP/1.0 501 Not Implemented\r\n",
    "Content-Type: text/plain\r\n",
    "Content-Length: 22\r\n",
    "\r\n",
    "501 Not Implemented\r\n",
    );
pub const NOT_IMPLEMENTED: &'static str = concat!(
    "HTTP/1.0 501 Not Implemented\r\n",
    "Content-Type: text/plain\r\n",
    "Content-Length: 22\r\n",
    "\r\n",
    );


impl<'a, 'b: 'a, S: StreamSocket> Response<'a, 'b, S> {
}
