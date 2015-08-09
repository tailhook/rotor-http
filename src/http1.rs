//! This is initial draft implementation of HTTP/1.x protocol. It's based
//! on `rotor::transports::greedy_stream` which is not very DDoS-safe. Also
//! this implementation has hard-coded limits on certain request parameters.
//! Otherwise implementation should be good enough for general consumption
//!
//!
use std::error::Error;

use rotor::transports::greedy_stream::{Transport, Protocol};
use rotor::buffer_util::find_substr;
use hyper::version::HttpVersion;
use hyper::method::Method;
use hyper::header::Headers;
use hyper::uri::RequestUri;
use httparse;


/// Note httparse requires we preallocate array of this size so be wise
pub const MAX_HEADERS_NUM: usize = 256;
/// This one is not preallocated, but too large buffer is of limited use
/// because of previous parameter.
pub const MAX_HEADERS_SIZE: usize = 16384;
/// This is not "enough for everyone" but we probably need some limit anyway.
/// Note that underlying `netbuf` impl limited to a little less than 4GiB
pub const MAX_BODY_SIZE: usize = 104_856_700;


pub enum Client {
    Initial,
    ReadHeaders,
    ReadFixedSize(Request, usize),
    ReadChunked(Request, usize),
    KeepAlive,
}

#[derive(Debug)]
pub struct Request {
    version: HttpVersion,
    method: Method,
    uri: RequestUri,
    headers: Headers,
    body: Vec<u8>,
}

fn parse_headers(transport: &mut Transport)
    -> Result<Client, Box<Error+Send+Sync>>
{
    use self::Client::*;
    use hyper::version::HttpVersion::*;

    let mut buf = transport.input();
    let headers_end = match find_substr(&buf[..], b"\r\n\r\n") {
        Some(x) => x,
        None => { return Ok(ReadHeaders); }
    };
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS_NUM];
    let mut raw = httparse::Request::new(&mut headers);
    match raw.parse(&buf[..]) {
        Ok(httparse::Status::Complete(x)) => {
            assert!(x == headers_end+4);
        }
        Ok(_) => unreachable!(),
        Err(_) => {
            return Err(From::from("Header syntax mismatch"));
        }
    }
    let req = Request {
        version: if raw.version.unwrap() == 1 { Http11 } else { Http10 },
        method: try!(raw.method.unwrap().parse()),
        uri: try!(raw.path.unwrap().parse()),
        headers: try!(Headers::from_raw(raw.headers)),
        body: Vec::new(),
    };
    // TODO(tailhook) check content-length, connection: close, transfer-encoding
    println!("Request received {:?}", req);
    Err(From::from("Not Implemented"))
}

impl Protocol for Client {
    fn accepted() -> Self {
        Client::Initial
    }
    fn data_received(self, transport: &mut Transport) -> Option<Self> {
        use self::Client::*;
        match self {
            // We keep Initial and KeepAlive states separate from
            // ReadHeaders just for debugging and for potentially different
            // timeouts in future
            Initial|ReadHeaders|KeepAlive => {
                parse_headers(transport)
                .map_err(|e| debug!("Error parsing HTTP headers: {}", e))
                .ok()
            }
            _ => unimplemented!(),
        }
    }
}
