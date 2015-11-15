//! This is initial draft implementation of HTTP/1.x protocol. It's based
//! on `rotor::transports::greedy_stream` which is not very DDoS-safe. Also
//! this implementation has hard-coded limits on certain request parameters.
//! Otherwise implementation should be good enough for general consumption
//!
//!
use std::cmp;
use std::io::Write;
use std::error::Error;
use std::marker::PhantomData;
use std::mem::replace;

use time::now_utc;
use rotor::transports::stream::{Transport, Protocol};
use rotor::transports::StreamSocket;
use rotor::buffer_util::find_substr;
use rotor::async::Async;
use hyper::version::HttpVersion;
use hyper::method::Method;
use hyper::header::{Headers, Date, HttpDate, ContentLength};
use hyper::uri::RequestUri;
use hyper::status::StatusCode;
use httparse;


/// Note httparse requires we preallocate array of this size so be wise
pub const MAX_HEADERS_NUM: usize = 256;
/// This one is not preallocated, but too large buffer is of limited use
/// because of previous parameter.
pub const MAX_HEADERS_SIZE: usize = 16384;
/// This is not "enough for everyone" but we probably need some limit anyway.
/// Note that underlying `netbuf` impl limited to a little less than 4GiB
pub const MAX_BODY_SIZE: usize = 104_856_700;


pub trait Handler<C> {
    /// Dispatched when request arrives.
    ///
    /// We don't support POST body yet, so this is only one callback, but will
    /// probably be split into many in future
    fn request(_request: Request, _response: &mut ResponseBuilder,
               _ctx: &mut C)
    {
        // The 404 or BadRequest for all requests
    }
}

#[derive(Debug)]
pub enum ResponseFsm {
    Head {
        status: StatusCode,
        version: HttpVersion,
        headers: Headers,
    },
    // TODO(tailhook) WritingFixed { bytes_remaining: u64 }
    // TODO(tailhook) WritingChunked
    End,
}

pub struct ResponseBuilder<'a, 'b: 'a>{
    state: ResponseFsm,
    transport: &'a mut Transport<'b>,
}

impl <'a, 'b>ResponseBuilder<'a, 'b> {
    pub fn new(req: &Request, transport: &'a mut Transport<'b>)
        -> ResponseBuilder<'a, 'b>
    {
        ResponseBuilder {
            state: ResponseFsm::Head {
                status: if req.method == Method::Get {
                    StatusCode::NotFound
                } else {
                    StatusCode::BadRequest
                },
                version: req.version,
                headers: Headers::new(),
            },
            transport: transport,
        }
    }
}

/// A connection with a client.
///
/// The `Initial`, `KeepAlive` and `ReadHeaders` states are kept separate for
/// debugging and different timeouts in future eventuallly.
pub enum Client<C, H: Handler<C>> {
    /// The initial state of a connection.
    Initial,
    /// The state after some headers have been read.
    ReadHeaders, // TODO(tailhook) 100 Expect?
    /// Not yet supported.
    Processing(H, PhantomData<*const C>),
    /// Reading a request body with a fixed size.
    ///
    /// The `usize` gives the number of remaining bytes.
    ReadFixedSize(Request, usize),
    // TODO ReadChunked(Request, usize),
    /// A connection in idle state.
    KeepAlive,
}

#[derive(Debug)]
pub struct Request {
    pub version: HttpVersion,
    pub method: Method,
    pub uri: RequestUri,
    pub headers: Headers,
    pub body: Vec<u8>,
}

impl Request {
    fn content_length(&self) -> Option<usize> {
        if let Some(length_header) = self.headers.get::<ContentLength>() {
            return Some(**length_header as usize)
        }
        None
    }
}

fn parse_headers(transport: &mut Transport)
    -> Result<Option<Request>, Box<Error+Send+Sync>>
{
    use hyper::version::HttpVersion::*;

    let mut buf = transport.input();
    let headers_end = match find_substr(&buf[..], b"\r\n\r\n") {
        Some(x) => x,
        None => { return Ok(None); }
    };
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS_NUM];
    let req = {
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
        Request {
            version: if raw.version.unwrap() == 1 { Http11 } else { Http10 },
            method: try!(raw.method.unwrap().parse()),
            uri: try!(raw.path.unwrap().parse()),
            headers: try!(Headers::from_raw(raw.headers)),
            body: Vec::new(),
        }
    };
    buf.consume(headers_end+4);
    Ok(Some(req))
}

impl<C, H: Handler<C>> Protocol<C> for Client<C, H> {
    fn accepted<S: StreamSocket>(_conn: &mut S, _context: &mut C)
        -> Option<Self>
    {
        Some(Client::Initial)
    }
    fn data_received(self, transport: &mut Transport, ctx: &mut C)
        -> Async<Self, ()>
    {
        use self::Client::*;
        match self {
            Initial | ReadHeaders | KeepAlive => {
                match parse_headers(transport) {
                    Err(_) => Async::Stop,
                    Ok(None) => Async::Continue(ReadHeaders, ()),
                    Ok(Some(req)) => {
                        if let Some(length) = req.content_length() {
                            Async::Continue(ReadFixedSize(req, length), ())
                        } else {
                            let mut bld = ResponseBuilder::new(&req,
                                                               transport);
                            <H as Handler<C>>::request(req, &mut bld, ctx);
                            bld.default_body();
                            Async::Continue(Client::KeepAlive, ())
                        }
                    }
                }
            }
            ReadFixedSize(mut req, mut length) => {
                {
                    let mut buf = transport.input();
                    let read_length = cmp::min(length, buf.len());
                    length -= read_length;
                    req.body.extend(&buf[..read_length]);
                    buf.consume(read_length);
                }
                if length != 0 {
                    Async::Continue(ReadFixedSize(req, length), ())
                } else {
                    let mut bld = ResponseBuilder::new(&req, transport);
                    <H as Handler<C>>::request(req, &mut bld, ctx);
                    bld.default_body();
                    Async::Continue(Client::KeepAlive, ())
                }
            }
            _ => unimplemented!()
        }
    }
}

impl<'a, 'b> ResponseBuilder<'a, 'b> {

    /// Write request body in a single go
    pub fn put_body<B:AsRef<[u8]>>(&mut self, body: B) {
        use self::ResponseFsm::*;
        match replace(&mut self.state, End) {
            Head { status, version, mut headers } => {
                let body = body.as_ref();
                let out = self.transport.output();
                write!(out, "{} {}\r\n", version, status).unwrap();
                if !headers.has::<Date>() {
                    headers.set(Date(HttpDate(now_utc())));
                }
                headers.set(ContentLength(body.len() as u64));
                write!(out, "{}\r\n", headers).unwrap();
                out.extend(body);
            }
            state => {
                panic!("Too late to send body in state: {:?}", state);
            }
        }
    }

    pub fn set_status(&mut self, new_status: StatusCode) {
        use self::ResponseFsm::*;
        match self.state {
            Head { ref mut status, .. } => {
                *status = new_status;
            }
            ref state => {
                panic!("Too late to set status in state: {:?}", state);
            }
        }
    }

    pub fn default_body(&mut self) {
        use self::ResponseFsm::*;
        match self.state {
            Head { status, .. } => {
                // TODO(tailhook) maybe assert on 200 Ok status?
                self.put_body(&format!("{}", status));
                self.state = End;
            }
            End => {}
        }
    }
}
