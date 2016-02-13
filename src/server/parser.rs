use std::any::Any;
use std::io::Write;
use std::cmp::min;
use std::str::from_utf8;
use std::marker::PhantomData;

use rotor::mio::tcp::TcpStream;
use rotor_stream::MAX_BUF_SIZE;
use rotor::Scope;
use rotor_stream::{Protocol, StreamSocket, Deadline, Expectation as E};
use rotor_stream::{Request, Transport, Exception};

use httparse;
use headers;
use recvmode::RecvMode;
use message::{MessageState};
use super::{MAX_HEADERS_SIZE, MAX_HEADERS_NUM, MAX_CHUNK_HEAD};
use super::{Response};
use super::protocol::{Server};
use super::context::Context;
use super::request::Head;
use super::body::BodyKind;
use super::response::state;
use Version;

use self::ErrorCode::*;


struct ReadBody<M: Server> {
    machine: Option<M>,
    deadline: Deadline,
    response: MessageState,
    progress: BodyProgress,
    connection_close: bool,
}

pub enum BodyProgress {
    /// Buffered fixed-size request (bytes left)
    BufferFixed(usize),
    /// Buffered request with chunked encoding
    /// (limit, bytes buffered, bytes left for current chunk)
    BufferChunked(usize, usize, usize),
    /// Progressive fixed-size request (size hint, bytes left)
    ProgressiveFixed(usize, u64),
    /// Progressive with chunked encoding
    /// (hint, offset, bytes left for current chunk)
    ProgressiveChunked(usize, usize, u64),
}

pub struct Parser<M, S>(ParserImpl<M>, PhantomData<*const S>)
    where M: Server, S: StreamSocket;

enum ParserImpl<M: Server> {
    Idle,
    ReadHeaders,
    ReadingBody(ReadBody<M>),
    /// Close connection after buffer is flushed. In other cases -> Idle
    Processing(M, MessageState, bool, Deadline),
    DoneResponse,
}

pub enum ErrorCode {
    BadRequest,
    PayloadTooLarge,
    RequestTimeout,
    RequestHeaderFieldsTooLarge,
}

impl<M: Server, S: StreamSocket> Parser<M, S> {
    fn flush(scope: &mut Scope<M::Context>) -> Request<Parser<M, S>>
    {
        Some((ParserImpl::DoneResponse.wrap(), E::Flush(0),
              Deadline::now() + scope.byte_timeout()))
    }
    fn error<'x>(scope: &mut Scope<M::Context>, mut response: Response<'x>,
        code: ErrorCode)
        -> Request<Parser<M, S>>
    {
        if !response.is_started() {
            scope.emit_error_page(code, &mut response);
        }
        response.finish();
        Some((ParserImpl::DoneResponse.wrap(), E::Flush(0),
              Deadline::now() + scope.byte_timeout()))
    }
    fn raw_error<'x>(scope: &mut Scope<M::Context>,
        transport: &mut Transport<<Self as Protocol>::Socket>,
        code: ErrorCode)
        -> Request<Parser<M, S>>
    {
        let resp = Response::new(transport.output(),
            Version::Http10, false, true);
        Parser::error(scope, resp, code)
    }
    fn complete<'x>(scope: &mut Scope<M::Context>, machine: Option<M>,
        response: Response<'x>, connection_close: bool, deadline: Deadline)
        -> Request<Parser<M, S>>
    {
        match machine {
            Some(m) => {
                Some((ParserImpl::Processing(m, state(response),
                        connection_close, deadline)
                      .wrap(),
                    E::Sleep, deadline))
            }
            None => {
                // TODO(tailhook) probably we should do something better than
                // an assert?
                assert!(response.is_complete());
                if connection_close {
                    Parser::flush(scope)
                } else {
                    ParserImpl::Idle.request(scope)
                }
            }
        }
    }
}

fn start_headers<C: Context, M: Server, S: StreamSocket>(scope: &mut Scope<C>)
    -> Request<Parser<M, S>>
{
    Some((ParserImpl::ReadHeaders.wrap(),
          E::Delimiter(0, b"\r\n\r\n", MAX_HEADERS_SIZE),
          Deadline::now() + scope.byte_timeout()))
}

fn start_body(mode: RecvMode, body: BodyKind) -> BodyProgress {
    use recvmode::RecvMode::*;
    use super::body::BodyKind::*;
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

fn scan_headers(version: Version, headers: &[httparse::Header])
    -> Result<(BodyKind, bool, bool), ()>
{
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
    use super::body::BodyKind::*;
    let mut has_content_length = false;
    let mut close = version == Version::Http10;
    let mut expect_continue = false;
    let mut result = BodyKind::Fixed(0);
    for header in headers.iter() {
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
            if result != Chunked {
                let s = try!(from_utf8(header.value).map_err(|_| ()));
                let len = try!(s.parse().map_err(|_| ()));
                result = Fixed(len);
            } else {
                // tralsfer-encoding has preference and don't allow keep-alive
                close = true;
            }
        } else if headers::is_connection(header.name) {
            if header.value.split(|&x| x == b',').any(headers::is_close) {
                close = true;
            }
        } else if headers::is_expect(header.name) {
            if headers::is_continue(header.value) {
                expect_continue = true;
            }
        }
    }
    Ok((result, expect_continue, close))
}

enum HeaderResult<M: Server> {
    Okay(ReadBody<M>),
    NormError(bool, Version, ErrorCode),
    FatalError(ErrorCode),
    ForceClose,
}

// Parses headers
//
// On error returns bool, which is true if keep-alive connection can be
// carried on.
fn parse_headers<S, M>(transport: &mut Transport<S>, end: usize,
    scope: &mut Scope<M::Context>) -> HeaderResult<M>
    where M: Server, S: StreamSocket,
{
    use self::HeaderResult::*;
    let client = Any::downcast_ref::<TcpStream>(transport.socket())
        .and_then(|x| x.peer_addr().ok());
    let result = {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS_NUM];
        let (input, output) = transport.buffers();
        let (version, method, path, headers) = {
            let mut raw = httparse::Request::new(&mut headers);
            match raw.parse(&input[..end+4]) {
                Ok(httparse::Status::Complete(x)) => {
                    debug_assert!(x == end+4);
                    (if raw.version.unwrap() == 1 { Version::Http11 }
                             else { Version::Http10 },
                     raw.method.unwrap(),
                     raw.path.unwrap(),
                     raw.headers)
                }
                Ok(_) => unreachable!(),
                Err(_) => {
                    return FatalError(BadRequest);
                }
            }
        };
        let is_head = method == "HEAD";

        let (body, expect, close) = match scan_headers(version, headers) {
            Ok(triple) => triple,
            Err(()) => {
                return FatalError(BadRequest);
            }
        };
        let head = Head {
            client: client,
            version: version,
            method: method,
            scheme: "http",
            path: path,
            headers: headers,
            body_kind: body,
        };
        let (head_result, state) = {
            let mut res = Response::new(output, version, is_head, close);
            (M::headers_received(head, &mut res, scope),
             state(res))
        };
        match (head_result, body) {
            (Some((_, RecvMode::Buffered(x), _)), _) if x >= MAX_BUF_SIZE
            => panic!("Can't buffer {} bytes, max {}", x, MAX_BUF_SIZE),
            (Some((_, RecvMode::Buffered(y), _)), BodyKind::Fixed(x))
            if x >= y as u64 => {
                return FatalError(PayloadTooLarge)
            }
            (Some((m, mode, dline)), body) => {
                if expect && !state.is_started() {
                    write!(output, "{} 100 Continue\r\n\r\n", version)
                        .unwrap();
                }
                Okay(ReadBody {
                    machine: Some(m),
                    deadline: dline,
                    progress: start_body(mode, body),
                    response: state,
                    connection_close: close,
                })
            }
            (None, _) => {
                if state.is_started() {
                    // Can't do anything as response is already started
                    // just drop connection as fast as possible
                    ForceClose
                } else if close || body != BodyKind::Fixed(0) {
                    FatalError(BadRequest)
                } else {
                    NormError(is_head, version, BadRequest)
                }
            }
        }
    };
    transport.input().consume(end+4);
    result
}

impl<M: Server> ParserImpl<M>
{
    fn wrap<S: StreamSocket>(self) -> Parser<M, S>
    {
        Parser(self, PhantomData)
    }
    fn request<C, S>(self, scope: &mut Scope<C>) -> Request<Parser<M, S>>
        where C: Context, S: StreamSocket
    {
        use rotor_stream::Expectation::*;
        use self::ParserImpl::*;
        use self::BodyProgress::*;
        let (exp, dline) = match self {
            Idle => (Bytes(0), None),
            ReadHeaders => (Delimiter(0, b"\r\n\r\n", MAX_HEADERS_SIZE), None),
            ReadingBody(ref b) => {
                let exp = match *&b.progress {
                    BufferFixed(x) => Bytes(x),
                    BufferChunked(_, off, 0)
                    => Delimiter(off, b"\r\n", off+MAX_CHUNK_HEAD),
                    BufferChunked(_, off, y) => Bytes(off + y),
                    ProgressiveFixed(hint, left)
                    => Bytes(min(hint as u64, left) as usize),
                    ProgressiveChunked(_, off, 0)
                    => Delimiter(off, b"\r\n", off+MAX_CHUNK_HEAD),
                    ProgressiveChunked(hint, off, left)
                    => Bytes(min(hint as u64, off as u64 +left) as usize)
                };
                (exp, Some(b.deadline))
            }
            Processing(..) => unreachable!(),
            /// TODO(tailhook) fix output timeout
            DoneResponse => (Flush(0), None),
        };

        let byte_dline = Deadline::now() + scope.byte_timeout();
        let deadline = dline.map_or_else(
            || byte_dline,
            |x| min(byte_dline, x));
        Some((self.wrap(), exp, deadline))
    }
}

impl<S: StreamSocket, M: Server> Protocol for Parser<M, S> {
    type Context = M::Context;
    type Socket = S;
    type Seed = ();
    fn create(_seed: (), _sock: &mut Self::Socket,
        scope: &mut Scope<M::Context>)
        -> Request<Self>
    {
        Some((ParserImpl::Idle.wrap(), E::Bytes(1),
            Deadline::now() + scope.byte_timeout()))
    }
    fn bytes_read(self, transport: &mut Transport<S>,
                  end: usize, scope: &mut Scope<M::Context>)
        -> Request<Self>
    {
        use self::ParserImpl::*;
        use self::BodyProgress::*;
        use self::HeaderResult::*;
        match self.0 {
            Idle => {
                start_headers(scope)
            }
            ReadHeaders => {
                match parse_headers(transport, end, scope) {
                    Okay(body) => {
                        ReadingBody(body).request(scope)
                    }
                    NormError(is_head, version, status) => {
                        let mut resp = Response::new(
                            transport.output(), version, is_head, false);
                        scope.emit_error_page(status, &mut resp);
                        if resp.finish() {
                            Idle.request(scope)
                        } else {
                            Parser::flush(scope)
                        }
                    }
                    FatalError(status) => {
                        // TODO(tailhook) force Connection: close on the
                        // response
                        let mut resp = Response::new(
                            transport.output(), Version::Http10, false, true);
                        scope.emit_error_page(status, &mut resp);
                        Parser::flush(scope)
                    }
                    ForceClose => None,
                }
            }
            ReadingBody(rb) => {
                let (inp, out) = transport.buffers();
                let mut resp = rb.response.with(out);
                let (m, progress) = match rb.progress {
                    BufferFixed(x) => {
                        let m = rb.machine.and_then(
                            |m| m.request_received(
                                            &inp[..x], &mut resp, scope));
                        inp.consume(x);
                        (m, None)
                    }
                    BufferChunked(limit, off, 0) => {
                        let clen_end = inp[off..end].iter()
                            .position(|&x| x == b';')
                            .map(|x| x + off).unwrap_or(end);
                        let val_opt = from_utf8(&inp[off..clen_end]).ok()
                            .and_then(|x| u64::from_str_radix(x, 16).ok());
                        match val_opt {
                            Some(0) => {
                                inp.remove_range(off..end+2);
                                let m = rb.machine.and_then(
                                    |m| m.request_received(
                                        &inp[..off], &mut resp, scope));
                                inp.consume(off);
                                (m, None)
                            }
                            Some(chunk_len) => {
                                if off as u64 + chunk_len > limit as u64 {
                                    inp.consume(end+2);
                                    rb.machine.map(
                                        |m| m.bad_request(&mut resp, scope));
                                    return Parser::error(scope, resp,
                                                         BadRequest);
                                }
                                inp.remove_range(off..end+2);
                                (rb.machine,
                                    Some(BufferChunked(limit, off,
                                                  chunk_len as usize)))
                            }
                            None => {
                                inp.consume(end+2);
                                rb.machine.map(
                                    |m| m.bad_request(&mut resp, scope));
                                return Parser::error(scope, resp,
                                                     BadRequest);
                            }
                        }
                    }
                    BufferChunked(limit, off, bytes) => {
                        debug_assert!(bytes == end);
                        (rb.machine, Some(BufferChunked(limit, off+bytes, 0)))
                    }
                    ProgressiveFixed(hint, mut left) => {
                        let real_bytes = min(inp.len() as u64, left) as usize;
                        let m = rb.machine.and_then(
                            |m| m.request_chunk(
                                &inp[..real_bytes], &mut resp, scope));
                        inp.consume(real_bytes);
                        left -= real_bytes as u64;
                        if left == 0 {
                            let m = m.and_then(
                                |m| m.request_end(&mut resp, scope));
                            (m, None)
                        } else {
                            (m, Some(ProgressiveFixed(hint, left)))
                        }
                    }
                    ProgressiveChunked(hint, off, 0) => {
                        let clen_end = inp[off..end].iter()
                            .position(|&x| x == b';')
                            .map(|x| x + off).unwrap_or(end);
                        let val_opt = from_utf8(&inp[off..clen_end]).ok()
                            .and_then(|x| u64::from_str_radix(x, 16).ok());
                        match val_opt {
                            Some(0) => {
                                inp.remove_range(off..end+2);
                                let m = rb.machine.and_then(
                                    |m| m.request_received(
                                        &inp[..off], &mut resp, scope));
                                inp.consume(off);
                                (m, None)
                            }
                            Some(chunk_len) => {
                                inp.remove_range(off..end+2);
                                (rb.machine,
                                    Some(ProgressiveChunked(hint, off,
                                                  chunk_len)))
                            }
                            None => {
                                inp.consume(end+2);
                                rb.machine.map(
                                    |m| m.bad_request(&mut resp, scope));
                                return Parser::error(scope, resp, BadRequest);
                            }
                        }
                    }
                    ProgressiveChunked(hint, off, mut left) => {
                        let ln = min(off as u64 + left, inp.len() as u64) as usize;
                        left -= (ln - off) as u64;
                        if ln < hint {
                            (rb.machine,
                                Some(ProgressiveChunked(hint, ln, left)))
                        } else {
                            let m = rb.machine.and_then(
                                |m| m.request_chunk(&inp[..ln],
                                    &mut resp, scope));
                            inp.consume(ln);
                            (m, Some(ProgressiveChunked(hint, 0, left)))
                        }
                    }
                };
                match progress {
                    Some(p) => {
                        ReadingBody(ReadBody {
                            machine: m,
                            deadline: rb.deadline,
                            progress: p,
                            response: state(resp),
                            connection_close: rb.connection_close,
                        }).request(scope)
                    }
                    None => Parser::complete(scope, m, resp,
                        rb.connection_close, rb.deadline)
                }
            }
            // Spurious event?
            me @ DoneResponse => me.request(scope),
            Processing(m, r, c, dline) => {
                Some((Processing(m, r, c, dline).wrap(), E::Sleep, dline))
            }
        }
    }
    fn bytes_flushed(self, _transport: &mut Transport<S>,
                     scope: &mut Scope<Self::Context>)
        -> Request<Self>
    {
        match self.0 {
            ParserImpl::DoneResponse => None,
            me => me.request(scope),
        }
    }
    fn exception(self, transport: &mut Transport<S>, exc: Exception,
        scope: &mut Scope<Self::Context>)
        -> Request<Self>
    {
        use self::ParserImpl::*;
        use self::BodyProgress::*;
        use rotor_stream::Exception::*;
        match exc {
            LimitReached => {
                match self.0 {
                    ReadHeaders => {
                        Parser::raw_error(scope, transport,
                            RequestHeaderFieldsTooLarge)
                    }
                    ReadingBody(rb) => {
                        assert!(matches!(rb.progress,
                            ProgressiveChunked(_, _, 0) |
                            BufferChunked(_, _, 0)));
                        let mut resp = rb.response.with(transport.output());
                        rb.machine.map(|m| m.bad_request(&mut resp, scope));
                        Parser::error(scope, resp, BadRequest)
                    }
                    _ => unreachable!(),
                }
            }
            EndOfStream => {
                match self.0 {
                    ReadingBody(rb) => {
                        // Incomplete request
                        let mut resp = rb.response.with(
                            transport.output());
                        rb.machine.map(
                            |m| m.bad_request(&mut resp, scope));
                        Parser::error(scope, resp, BadRequest)
                    }
                    Processing(..) => unreachable!(),
                    Idle | ReadHeaders | DoneResponse => None,
                }
            }
            ReadError(_) => None,
            WriteError(_) => None,
        }
    }
    fn timeout(self, transport: &mut Transport<S>,
        scope: &mut Scope<Self::Context>)
        -> Request<Self>
    {
        use self::ParserImpl::*;
        match self.0 {
            Idle | DoneResponse => None,
            ReadHeaders => {
                Parser::raw_error(scope, transport, RequestTimeout)
            }
            ReadingBody(rb) => {
                let mut resp = rb.response.with(transport.output());
                let res = rb.machine.and_then(|m| m.timeout(&mut resp, scope));
                match res {
                    Some((m, deadline)) => {
                        ReadingBody(ReadBody {
                            machine: Some(m),
                            deadline: deadline,
                            progress: rb.progress,
                            response: state(resp),
                            connection_close: rb.connection_close,
                        }).request(scope)
                    }
                    None => Parser::error(scope, resp, RequestTimeout),
                }
            }
            Processing(m, respimp, close, _) => {
                let mut resp = respimp.with(transport.output());
                match m.timeout(&mut resp, scope) {
                    Some((m, dline)) => {
                        Parser::complete(scope, Some(m), resp, close, dline)
                    }
                    None => Parser::error(scope, resp, RequestTimeout),
                }
            }
        }
    }
    fn wakeup(self, transport: &mut Transport<S>,
        scope: &mut Scope<Self::Context>)
        -> Request<Self>
    {
        use self::ParserImpl::*;
        match self.0 {
            me@Idle | me@ReadHeaders | me@DoneResponse => me.request(scope),
            ReadingBody(rb) => {
                let mut resp = rb.response.with(transport.output());
                let m = rb.machine.and_then(|m| m.wakeup(&mut resp, scope));
                ReadingBody(ReadBody {
                    machine: m,
                    deadline: rb.deadline,
                    progress: rb.progress,
                    response: state(resp),
                    connection_close: rb.connection_close,
                }).request(scope)
            }
            Processing(m, respimp, close, dline) => {
                let mut resp = respimp.with(transport.output());
                let mres = m.wakeup(&mut resp, scope);
                Parser::complete(scope, mres, resp, close, dline)
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::io::{self, Read, Write};
    use rotor::mio::{Evented, Token, Selector};
    use rotor::{Scope, EventSet, PollOpt};
    use rotor_stream::Deadline;
    use super::Parser;
    use super::super::{Server, Head, Response, RecvMode};

    struct Proto(usize);
    struct MockStream;

    impl Server for Proto {
        type Context = ();
        fn headers_received(_head: Head, _response: &mut Response,
            _scope: &mut Scope<Self::Context>)
            -> Option<(Self, RecvMode, Deadline)>
        { unimplemented!(); }
        fn request_received(self, _data: &[u8], _response: &mut Response,
            _scope: &mut Scope<Self::Context>) -> Option<Self>
        { unimplemented!(); }
        fn request_chunk(self, _chunk: &[u8], _response: &mut Response,
            _scope: &mut Scope<Self::Context>) -> Option<Self>
        { unimplemented!(); }
        fn request_end(self, _response: &mut Response,
            _scope: &mut Scope<Self::Context>) -> Option<Self>
        { unimplemented!(); }
        fn timeout(self, _response: &mut Response,
            _scope: &mut Scope<Self::Context>) -> Option<(Self, Deadline)>
        { unimplemented!(); }
        fn wakeup(self, _response: &mut Response,
            _scope: &mut Scope<Self::Context>) -> Option<Self>
        { unimplemented!(); }
    }

    impl Read for MockStream {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize>
        { unimplemented!() }
    }
    impl Write for MockStream {
        fn write(&mut self, _: &[u8]) -> io::Result<usize>
        { unimplemented!() }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    impl Evented for MockStream {
        fn register(&self, _selector: &mut Selector,
            _token: Token, _interest: EventSet, _opts: PollOpt)
            -> io::Result<()>
        { unimplemented!() }
        fn reregister(&self, _selector: &mut Selector, _token: Token,
            _interest: EventSet, _opts: PollOpt) -> io::Result<()>
        { unimplemented!() }
        fn deregister(&self, _selector: &mut Selector) -> io::Result<()>
        { unimplemented!() }
    }

    #[test]
    fn parser_size() {
        // Just to keep track of size of structure
        assert_eq!(::std::mem::size_of::<Parser<Proto, MockStream>>(), 104);
    }
}
