use std::marker::PhantomData;
use std::str::from_utf8;

use rotor::Scope;
use rotor_stream::{Protocol, StreamSocket, Deadline, Expectation as E};
use rotor_stream::{Request as Task, Transport};
use rotor_stream::Buf;
use hyper::version::HttpVersion as Version;
use httparse;

use super::{MAX_HEADERS_SIZE, MAX_HEADERS_NUM};
use super::{Client, Context};
use super::head::Head;
use super::request::{Request, state};
use super::head::BodyKind;
use message::{MessageState, Message};
use recvmode::RecvMode;
use headers;


pub enum BodyProgress {
    /// Buffered fixed-size request (bytes left)
    BufferFixed(usize),
    /// Buffered request till end of input (byte limit)
    BufferEOF(usize),
    /// Buffered request with chunked encoding
    /// (limit, bytes buffered, bytes left for current chunk)
    BufferChunked(usize, usize, usize),
    /// Progressive fixed-size request (size hint, bytes left)
    ProgressiveFixed(usize, u64),
    /// Progressive till end of input (size hint)
    ProgressiveEOF(usize),
    /// Progressive with chunked encoding
    /// (hint, offset, bytes left for current chunk)
    ProgressiveChunked(usize, usize, u64),
}

pub struct Parser<M, S>(ParserImpl<M>, PhantomData<*const S>)
    where M: Client, S: StreamSocket;

enum ParserImpl<M: Client> {
    WriteRequest(M),
    ReadHeaders {
        machine: M,
        request: MessageState,
        is_head: Option<bool>,
    },
    Response {
        progress: BodyProgress,
        machine: M,
        deadline: Deadline,
        request: MessageState,
    },
    Idle,
}

fn scan_headers(is_head: bool, code: u16, headers: &[httparse::Header])
    -> Result<(BodyKind, usize, bool), ()>
{
    /// Implements the body length algorithm for requests:
    /// http://httpwg.github.io/specs/rfc7230.html#message.body.length
    ///
    /// Algorithm:
    ///
    /// 1. For HEAD, 1xx, 204, 304 -- no body
    /// 2. If last transfer encoding is chunked -> Chunked
    /// 3. If Content-Length -> Fixed
    /// 4. Else Eof
    use super::head::BodyKind::*;
    let mut has_content_length = false;
    let mut header_iter = headers.iter().enumerate();
    let mut close = false;
    if is_head || (code > 100 && code < 200) || code == 204 || code == 304 {
        for (idx, header) in header_iter {
            // TODO(tailhook) check for transfer encoding and content-length
            if headers::is_connection(header.name) {
                if header.value.split(|&x| x == b',').any(headers::is_close) {
                    close = true;
                }
            } else if header.name == "" {
                return Ok((Fixed(0), idx, close));
            }
        }
        return Ok((Fixed(0), headers.len(), close))
    }
    let mut result = BodyKind::Eof;
    for (idx, header) in header_iter {
        if headers::is_transfer_encoding(header.name) {
            if let Some(enc) = header.value.split(|&x| x == b',').last() {
                if headers::is_chunked(enc) {
                    if has_content_length {
                        // override but don't allow keep-alive
                        close = true;
                    }
                    result = Chunked;
                }
            }
        } else if headers::is_content_length(header.name) {
            if has_content_length {
                // duplicate content_length
                return Err(());
            }
            has_content_length = true;
            let s = try!(from_utf8(header.value).map_err(|_| ()));
            let len = try!(s.parse().map_err(|_| ()));
            result = Fixed(len);
        } else if headers::is_connection(header.name) {
            if header.value.split(|&x| x == b',').any(headers::is_close) {
                close = true;
            }
        } else if header.name == "" {
            return Ok((result, idx, close))
        }
    }
    return Ok((result, headers.len(), close))
}
fn start_body(mode: RecvMode, body: BodyKind) -> BodyProgress {
    use recvmode::RecvMode::*;
    use super::head::BodyKind::*;
    use self::BodyProgress::*;

    match (mode, body) {
        // The size of Fixed(x) is checked in parse_headers
        (Buffered(_), Fixed(y)) => BufferFixed(y as usize),
        (Buffered(x), Chunked) => BufferChunked(x, 0, 0),
        (Progressive(x), Fixed(y)) => ProgressiveFixed(x, y),
        (Progressive(x), Chunked) => ProgressiveChunked(x, 0, 0),
        (_, Upgrade) => unimplemented!(),
    }
}

fn parse_headers<M>(buffer: &mut Buf, end: usize,
    proto: M, mut req: Request, is_head: bool, scope: &mut Scope<M::Context>)
    -> Result<ParserImpl<M>, ()>
    where M: Client
{
    let resp = {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS_NUM];
        let (ver, code, reason) = {
            let mut raw = httparse::Response::new(&mut headers);
            match raw.parse(&buffer[..end+4]) {
                Ok(httparse::Status::Complete(x)) => {
                    assert!(x == end);
                    let ver = raw.version.unwrap();
                    let code = raw.code.unwrap();
                    (ver, code, raw.reason.unwrap())
                }
                Ok(_) => unreachable!(),
                Err(_) => {
                    // Anything to do with error?
                    // Should more precise errors be here?
                    return Err(());
                }
            }
        };
        let (body, num_headers, close) = try!(scan_headers(
            is_head, code, &headers));
        let head = Head {
            version: if ver == 1
                { Version::Http11 } else { Version::Http10 },
            code: code,
            reason: reason,
            headers: &headers[..num_headers],
            body_kind: body,
            // For HTTP/1.0 we could implement Connection: Keep-Alive
            // but hopefully it's rare enough to ignore nowadays
            close: close || ver == 0,
        };
        let hdr = proto.headers_received(head, &mut req, scope);
        let (mach, mode, dline) = match hdr {
            Some(triple) => triple,
            None => return Err(()),
        };
        let progress = start_body(mode, body);
        ParserImpl::Response {
            machine: mach,
            deadline: dline,
            progress: progress,
            request: state(req),
        }
    };
    buffer.consume(end+4);
    Ok(resp)
}

impl<M, S> Protocol for Parser<M, S>
    where M: Client, S: StreamSocket
{
    type Context = M::Context;
    type Socket = S;
    type Seed = M;
    fn create(seed: Self::Seed, sock: &mut Self::Socket,
        scope: &mut Scope<Self::Context>)
        -> Task<Self>
    {
        Some((Parser(ParserImpl::WriteRequest(seed), PhantomData),
            E::Flush(0), // Become writable
            Deadline::now() + scope.byte_timeout()))

    }
    fn bytes_read(self, transport: &mut Transport<Self::Socket>,
        end: usize, scope: &mut Scope<Self::Context>)
        -> Task<Self>
    {
        use self::ParserImpl::*;
        match self.0 {
            WriteRequest(m) => unreachable!(),
            ReadHeaders { machine, request, is_head } => {
                let (inb, outb) = transport.buffers();
                let is_head = is_head.unwrap();
                let hdr = parse_headers(inb, end, machine,
                    request.with(outb), is_head, scope);
                match hdr {
                    Ok(me) => unimplemented!(), //Parser(me, PhantomData).request(),
                    Err(()) => None, // Close the connection
                }
            }
            Response { progress, machine, deadline, request }  => {
                unimplemented!();
            }
            Idle => {
                Some((Parser(Idle, PhantomData),
                      E::Sleep, Deadline::now() + scope.idle_timeout()))
            }
        }
    }
    fn bytes_flushed(self, transport: &mut Transport<Self::Socket>,
        scope: &mut Scope<Self::Context>)
        -> Task<Self>
    {
        use self::ParserImpl::*;
        match self.0 {
            WriteRequest(m) => {
                let mut req = Request::new(transport.output());
                match m.prepare_request(&mut req) {
                    Some(m) => {
                        Some((Parser(ReadHeaders {
                                machine: m,
                                is_head: req.1,
                                request: state(req),
                            }, PhantomData),
                        E::Delimiter(0, b"\r\n\r\n", MAX_HEADERS_SIZE),
                        Deadline::now() + scope.byte_timeout()))
                    }
                    None => unimplemented!(),
                }
            }
            ReadHeaders {..} => unreachable!(),
            Response { progress, machine, deadline, request }  => {
                unimplemented!();
            }
            Idle => {
                Some((Parser(Idle, PhantomData),
                      E::Sleep, Deadline::now() + scope.idle_timeout()))
            }
        }
    }
    fn timeout(self, transport: &mut Transport<Self::Socket>,
        scope: &mut Scope<Self::Context>)
        -> Task<Self>
    {
        unimplemented!();
    }
    fn wakeup(self, transport: &mut Transport<Self::Socket>,
        scope: &mut Scope<Self::Context>)
        -> Task<Self>
    {
        unimplemented!();
    }

}
