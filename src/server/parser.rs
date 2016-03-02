use std::any::Any;
use std::io::Write;
use std::cmp::min;
use std::str::from_utf8;
use std::fmt;
use std::marker::PhantomData;

use rotor::mio::tcp::TcpStream;
use rotor_stream::MAX_BUF_SIZE;
use rotor::{Scope, Time};
use rotor_stream::{Protocol, StreamSocket};
use rotor_stream::{Intent, Transport, Exception};

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


#[derive(Debug)]
struct ReadBody<M: Server> {
    machine: Option<M>,
    deadline: Time,
    response: MessageState,
    progress: BodyProgress,
    connection_close: bool,
}

#[derive(Debug)]
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

#[derive(Debug)]
enum ParserImpl<M: Server> {
    Idle,
    ReadHeaders(usize),
    ReadingBody(ReadBody<M>),
    /// Close connection after buffer is flushed. In other cases -> Idle
    Processing(M, MessageState, bool, Time),
    DoneResponse,
}

pub enum ErrorCode {
    BadRequest,
    PayloadTooLarge,
    RequestTimeout,
    RequestHeaderFieldsTooLarge,
}

impl<M: Server + fmt::Debug, S: StreamSocket> fmt::Debug for Parser<M, S> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(fmt)
    }
}

impl<M: Server, S: StreamSocket> Parser<M, S> {
    fn flush(scope: &mut Scope<M::Context>) -> Intent<Parser<M, S>>
    {
        Intent::of(ParserImpl::DoneResponse.wrap()).expect_flush()
            .deadline(scope.now() + scope.byte_timeout())
    }
    fn error<'x>(scope: &mut Scope<M::Context>, mut response: Response<'x>,
        code: ErrorCode)
        -> Intent<Parser<M, S>>
    {
        if !response.is_started() {
            scope.emit_error_page(code, &mut response);
        }
        response.finish();
        Intent::of(ParserImpl::DoneResponse.wrap()).expect_flush()
            .deadline(scope.now() + scope.byte_timeout())
    }
    fn raw_error(scope: &mut Scope<M::Context>,
        transport: &mut Transport<<Self as Protocol>::Socket>,
        code: ErrorCode)
        -> Intent<Parser<M, S>>
    {
        let resp = Response::new(transport.output(),
            Version::Http10, false, true);
        Parser::error(scope, resp, code)
    }
    fn complete<'x>(scope: &mut Scope<M::Context>, machine: Option<M>,
        response: Response<'x>, connection_close: bool, deadline: Time)
        -> Intent<Parser<M, S>>
    {
        match machine {
            Some(m) => {
                Intent::of(ParserImpl::Processing(m, state(response),
                        connection_close, deadline)
                      .wrap())
                .sleep().deadline(deadline)
            }
            None => {
                // TODO(tailhook) probably we should do something better than
                // an assert?
                assert!(response.is_complete());
                if connection_close {
                    Parser::flush(scope)
                } else {
                    ParserImpl::Idle.intent(scope)
                }
            }
        }
    }
}

fn start_headers<C: Context, M: Server, S: StreamSocket>(scope: &mut Scope<C>)
    -> Intent<Parser<M, S>>
{
    Intent::of(ParserImpl::ReadHeaders(0).wrap())
    .expect_delimiter(b"\r\n\r\n", MAX_HEADERS_SIZE)
    .deadline(scope.now() + scope.byte_timeout())
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
    Incomplete,
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
                Ok(..) => {
                    return Incomplete;
                }
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

#[inline]
fn consumed(off: usize) -> usize {
    // If buffer is not empty it has final '\r\n' at the
    // end, and we are going to search for the next pair
    // But the `off` is stored as a number of useful bytes in the buffer
    if off > 0 { off+2 } else { 0 }
}

impl<M: Server> ParserImpl<M>
{
    fn wrap<S: StreamSocket>(self) -> Parser<M, S>
    {
        Parser(self, PhantomData)
    }
    fn intent<C, S>(self, scope: &mut Scope<C>) -> Intent<Parser<M, S>>
        where C: Context, S: StreamSocket
    {
        use rotor_stream::Expectation::*;
        use self::ParserImpl::*;
        use self::BodyProgress::*;
        let (exp, dline) = match self {
            Idle => (Bytes(0), None),
            ReadHeaders(x) => (Bytes(x+1), None),
            ReadingBody(ref b) => {
                let exp = match *&b.progress {
                    BufferFixed(x) => Bytes(x),
                    BufferChunked(_, off, 0) => {
                        Delimiter(consumed(off), b"\r\n",
                                  consumed(off)+MAX_CHUNK_HEAD)
                    }
                    BufferChunked(_, off, y) => Bytes(off + y + 2),
                    ProgressiveFixed(hint, left)
                    => Bytes(min(hint as u64, left) as usize),
                    ProgressiveChunked(_, off, 0)
                    => Delimiter(off, b"\r\n", off+MAX_CHUNK_HEAD),
                    ProgressiveChunked(hint, off, left)
                    => Bytes(min(hint as u64, off as u64 +left) as usize + 2)
                };
                (exp, Some(b.deadline))
            }
            Processing(..) => unreachable!(),
            /// TODO(tailhook) fix output timeout
            DoneResponse => (Flush(0), None),
        };

        let byte_dline = scope.now() + scope.byte_timeout();
        let deadline = dline.map_or_else(
            || byte_dline,
            |x| min(byte_dline, x));
        Intent::of(self.wrap()).expect(exp).deadline(deadline)
    }
}

impl<S: StreamSocket, M: Server> Protocol for Parser<M, S> {
    type Context = M::Context;
    type Socket = S;
    type Seed = ();
    fn create(_seed: (), _sock: &mut Self::Socket,
        scope: &mut Scope<M::Context>)
        -> Intent<Self>
    {
        Intent::of(ParserImpl::Idle.wrap()).expect_bytes(1)
            .deadline(scope.now() + scope.byte_timeout())
    }
    fn bytes_read(self, transport: &mut Transport<S>,
                  end: usize, scope: &mut Scope<M::Context>)
        -> Intent<Self>
    {
        use self::ParserImpl::*;
        use self::BodyProgress::*;
        use self::HeaderResult::*;
        match self.0 {
            Idle => {
                start_headers(scope)
            }
            ReadHeaders(x) => {
                match parse_headers(transport, end, scope) {
                    Okay(body) => {
                        ReadingBody(body).intent(scope)
                    }
                    Incomplete => {
                        ReadHeaders(x+1).intent(scope)
                    }
                    NormError(is_head, version, status) => {
                        let mut resp = Response::new(
                            transport.output(), version, is_head, false);
                        scope.emit_error_page(status, &mut resp);
                        if resp.finish() {
                            Idle.intent(scope)
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
                    ForceClose => Intent::done(),
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
                        use httparse::Status::*;
                        let lenstart = consumed(off);
                        match httparse::parse_chunk_size(
                                &inp[lenstart..lenstart+end+2])
                        {
                            Ok(Complete((_, 0))) => {
                                inp.remove_range(off..lenstart+end+2);
                                let m = rb.machine.and_then(
                                    |m| m.request_received(
                                        &inp[..off], &mut resp, scope));
                                inp.consume(off);
                                (m, None)
                            }
                            Ok(Complete((_, chunk_len))) => {
                                if off as u64 + chunk_len > limit as u64 {
                                    inp.consume(lenstart+end+2);
                                    rb.machine.map(
                                        |m| m.bad_request(&mut resp, scope));
                                    return Parser::error(scope, resp,
                                                         BadRequest);
                                }
                                inp.remove_range(off..lenstart+end+2);
                                (rb.machine,
                                    Some(BufferChunked(limit, off,
                                                  chunk_len as usize)))
                            }
                            Ok(Partial) => unreachable!(),
                            Err(_) => {
                                inp.consume(lenstart+end+2);
                                rb.machine.map(
                                    |m| m.bad_request(&mut resp, scope));
                                return Parser::error(scope, resp,
                                                     BadRequest);
                            }
                        }
                    }
                    BufferChunked(limit, off, bytes) => {
                        debug_assert_eq!(off + bytes, end - 2);
                        // We keep final \r\n in the buffer, so we can cut
                        // it together with next chunk length
                        // (i.e. do not do `remove_range` twice)
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
                        use httparse::Status::*;
                        match httparse::parse_chunk_size(&inp[off..off+end+2])
                        {
                            Ok(Complete((_, 0))) => {
                                inp.remove_range(off..off+end+2);
                                let mut m = rb.machine;
                                if off > 0 {
                                    m = m.and_then(
                                        |m| m.request_chunk(
                                            &inp[..off], &mut resp, scope));
                                }
                                m = m.and_then(
                                    |m| m.request_end(&mut resp, scope));
                                inp.consume(off);
                                (m, None)
                            }
                            Ok(Complete((_, chunk_len))) => {
                                inp.remove_range(off..off+end+2);
                                (rb.machine,
                                    Some(ProgressiveChunked(hint, off,
                                                  chunk_len)))
                            }
                            Ok(Partial) => unreachable!(),
                            Err(_) => {
                                inp.consume(off+end+2);
                                rb.machine.map(
                                    |m| m.bad_request(&mut resp, scope));
                                return Parser::error(scope, resp, BadRequest);
                            }
                        }
                    }
                    ProgressiveChunked(hint, off, mut left) => {
                        let ln = if off as u64 + left == (end - 2) as u64 {
                            // in progressive chunked we remove final '\r\n'
                            // immediately to make code simpler
                            // may be optimized later
                            inp.remove_range(end-2..end);
                            off + left as usize
                        } else {
                            inp.len()
                        };
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
                        }).intent(scope)
                    }
                    None => Parser::complete(scope, m, resp,
                        rb.connection_close, rb.deadline)
                }
            }
            // Spurious event?
            me @ DoneResponse => me.intent(scope),
            Processing(m, r, c, dline) => {
                Intent::of(Processing(m, r, c, dline).wrap())
                    .sleep().deadline(dline)
            }
        }
    }
    fn bytes_flushed(self, _transport: &mut Transport<S>,
                     scope: &mut Scope<Self::Context>)
        -> Intent<Self>
    {
        match self.0 {
            ParserImpl::DoneResponse => Intent::done(),
            me => me.intent(scope),
        }
    }
    fn exception(self, transport: &mut Transport<S>, exc: Exception,
        scope: &mut Scope<Self::Context>)
        -> Intent<Self>
    {
        use self::ParserImpl::*;
        use self::BodyProgress::*;
        use rotor_stream::Exception::*;
        match exc {
            LimitReached => {
                match self.0 {
                    ReadHeaders(..) => {
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
                    Idle | ReadHeaders(..) | DoneResponse => Intent::done(),
                }
            }
            ReadError(_) => Intent::done(),
            WriteError(_) => Intent::done(),
        }
    }
    fn timeout(self, transport: &mut Transport<S>,
        scope: &mut Scope<Self::Context>)
        -> Intent<Self>
    {
        use self::ParserImpl::*;
        match self.0 {
            Idle | DoneResponse => Intent::done(),
            ReadHeaders(..) => {
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
                        }).intent(scope)
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
        -> Intent<Self>
    {
        use self::ParserImpl::*;
        match self.0 {
            me@Idle | me@ReadHeaders(..) | me@DoneResponse => me.intent(scope),
            ReadingBody(rb) => {
                let mut resp = rb.response.with(transport.output());
                let m = rb.machine.and_then(|m| m.wakeup(&mut resp, scope));
                ReadingBody(ReadBody {
                    machine: m,
                    deadline: rb.deadline,
                    progress: rb.progress,
                    response: state(resp),
                    connection_close: rb.connection_close,
                }).intent(scope)
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
    use std::default::Default;
    use std::time::Duration;
    use std::str::from_utf8;
    use test::Bencher;
    use rotor_test::{MemIo, MockLoop};
    use rotor_stream::{Stream, Accepted};
    use rotor::{Scope, Time, EventSet, Machine};
    use super::Parser;
    use super::super::{Server, Head, Response, RecvMode};

    #[derive(Debug, PartialEq, Eq, Default)]
    pub struct Context {
        progressive: bool,
        headers_received: usize,
        chunks_received: usize,
        body: String,
        requests_received: usize,
    }

    impl super::super::Context for Context {}

    #[derive(Debug, PartialEq, Eq)]
    pub enum Proto {
        Reading,
        Done,
    }

    impl Server for Proto {
        type Context = Context;
        fn headers_received(_head: Head, _response: &mut Response,
            scope: &mut Scope<Self::Context>)
            -> Option<(Self, RecvMode, Time)>
        {
            scope.headers_received += 1;
            if scope.progressive {
                Some((Proto::Reading, RecvMode::Progressive(1000),
                    scope.now() + Duration::new(10, 0)))
            } else {
                Some((Proto::Reading, RecvMode::Buffered(1000),
                    scope.now() + Duration::new(10, 0)))
            }
        }
        fn request_received(self, data: &[u8], _response: &mut Response,
            scope: &mut Scope<Self::Context>) -> Option<Self>
        {
            scope.body.push_str(from_utf8(data).unwrap());
            scope.requests_received += 1;
            Some(Proto::Done)
        }
        fn request_chunk(self, chunk: &[u8], _response: &mut Response,
            scope: &mut Scope<Self::Context>) -> Option<Self>
        {
            scope.body.push_str(from_utf8(chunk).unwrap());
            scope.chunks_received += 1;
            Some(Proto::Reading)
        }
        fn request_end(self, _response: &mut Response,
            scope: &mut Scope<Self::Context>) -> Option<Self>
        {
            scope.requests_received += 1;
            Some(Proto::Done)
        }
        fn timeout(self, _response: &mut Response,
            _scope: &mut Scope<Self::Context>) -> Option<(Self, Time)>
        { unimplemented!(); }
        fn wakeup(self, _response: &mut Response,
            _scope: &mut Scope<Self::Context>) -> Option<Self>
        { unimplemented!(); }
    }

    #[test]
    fn parser_size() {
        // Just to keep track of size of structure
        assert_eq!(::std::mem::size_of::<Parser<Proto, MemIo>>(), 88);
    }


    #[test]
    fn test_zero_body() {
        let mut io = MemIo::new();
        let mut lp = MockLoop::new(Default::default());
        io.push_bytes("GET / HTTP/1.1\r\nContent-Length: 0\r\n\
                       Connection: close\r\n\r\n".as_bytes());
        let m = Stream::<Parser<Proto, MemIo>>::accepted(
            io.clone(), &mut lp.scope(1)).expect_machine();
        m.ready(EventSet::readable(), &mut lp.scope(1))
            .expect_machine();
        assert_eq!(*lp.ctx(), Context {
            progressive: false,
            headers_received: 1,
            body: String::from(""),
            chunks_received: 0,
            requests_received: 1,
        });
    }

    #[test]
    fn test_partial_headers() {
        let mut io = MemIo::new();
        let mut lp = MockLoop::new(Default::default());
        io.push_bytes("GET / HTTP/1.1\r\nContent-".as_bytes());
        let m = Stream::<Parser<Proto, MemIo>>::accepted(
            io.clone(), &mut lp.scope(1)).expect_machine();
        let m = m.ready(EventSet::readable(), &mut lp.scope(1))
            .expect_machine();
        assert_eq!(*lp.ctx(), Context {
            progressive: false,
            headers_received: 0,
            body: String::new(),
            chunks_received: 0,
            requests_received: 0,
        });
        io.push_bytes("Length: 0\r\n\r\n".as_bytes());
        m.ready(EventSet::readable(), &mut lp.scope(1))
            .expect_machine();
        assert_eq!(*lp.ctx(), Context {
            progressive: false,
            headers_received: 1,
            body: String::new(),
            chunks_received: 0,
            requests_received: 1,
        });
    }

    #[test]
    fn test_empty_chunked() {
        let mut io = MemIo::new();
        let mut lp = MockLoop::new(Default::default());
        io.push_bytes("GET / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\
                       Connection: close\r\n\r\n".as_bytes());
        let m = Stream::<Parser<Proto, MemIo>>::accepted(
            io.clone(), &mut lp.scope(1)).expect_machine();
        let m = m.ready(EventSet::readable(), &mut lp.scope(1))
            .expect_machine();
        assert_eq!(*lp.ctx(), Context {
            progressive: false,
            headers_received: 1,
            body: String::new(),
            chunks_received: 0,
            requests_received: 0,
        });
        io.push_bytes("0\r\n\r\n".as_bytes());
        m.ready(EventSet::readable(), &mut lp.scope(1));
            //.expect_machine();
        assert_eq!(*lp.ctx(), Context {
            progressive: false,
            headers_received: 1,
            body: String::new(),
            chunks_received: 0,
            requests_received: 1,
        });
    }

    #[test]
    fn test_one_chunk() {
        let mut io = MemIo::new();
        let mut lp = MockLoop::new(Default::default());
        io.push_bytes("GET / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\
                       Connection: close\r\n\r\n".as_bytes());
        let m = Stream::<Parser<Proto, MemIo>>::accepted(
            io.clone(), &mut lp.scope(1)).expect_machine();
        let m = m.ready(EventSet::readable(), &mut lp.scope(1))
            .expect_machine();
        assert_eq!(*lp.ctx(), Context {
            progressive: false,
            headers_received: 1,
            body: String::new(),
            chunks_received: 0,
            requests_received: 0,
        });
        io.push_bytes("5\r\nrotor\r\n0\r\n\r\n".as_bytes());
        m.ready(EventSet::readable(), &mut lp.scope(1))
            .expect_machine();
        assert_eq!(*lp.ctx(), Context {
            progressive: false,
            headers_received: 1,
            body: String::from("rotor"),
            chunks_received: 0,
            requests_received: 1,
        });
    }

    #[test]
    fn test_chunked_encoding() {
        let mut io = MemIo::new();
        let mut lp = MockLoop::new(Default::default());
        io.push_bytes("GET / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\
                       Connection: close\r\n\r\n".as_bytes());
        let m = Stream::<Parser<Proto, MemIo>>::accepted(
            io.clone(), &mut lp.scope(1)).expect_machine();
        let m = m.ready(EventSet::readable(), &mut lp.scope(1))
            .expect_machine();
        assert_eq!(*lp.ctx(), Context {
            progressive: false,
            headers_received: 1,
            chunks_received: 0,
            body: String::new(),
            requests_received: 0,
        });
        io.push_bytes("4\r\n\
                       Wiki\r\n\
                       5\r\n\
                       pedia\r\n\
                       E\r\n in\r\n\
                       \r\n\
                       chunks.\r\n\
                       0\r\n\
                       \r\n".as_bytes());
        m.ready(EventSet::readable(), &mut lp.scope(1))
            .expect_machine();
        assert_eq!(*lp.ctx(), Context {
            progressive: false,
            headers_received: 1,
            chunks_received: 0,
            body: String::from("Wikipedia in\r\n\r\nchunks."),
            requests_received: 1,
        });
    }

    #[test]
    fn test_progressive_chunked() {
        let mut io = MemIo::new();
        let mut lp = MockLoop::new(
            Context { progressive: true, ..Default::default() });
        io.push_bytes("GET / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\
                       Connection: close\r\n\r\n".as_bytes());
        let m = Stream::<Parser<Proto, MemIo>>::accepted(
            io.clone(), &mut lp.scope(1)).expect_machine();
        let m = m.ready(EventSet::readable(), &mut lp.scope(1))
            .expect_machine();
        assert_eq!(*lp.ctx(), Context {
            progressive: true,
            headers_received: 1,
            chunks_received: 0,
            body: String::new(),
            requests_received: 0,
        });
        io.push_bytes("4\r\n\
                       Wiki\r\n\
                       5\r\n\
                       pedia\r\n\
                       E\r\n in\r\n\
                       \r\n\
                       chunks.\r\n\
                       0\r\n\
                       \r\n".as_bytes());
        m.ready(EventSet::readable(), &mut lp.scope(1))
            .expect_machine();
        assert_eq!(*lp.ctx(), Context {
            progressive: true,
            headers_received: 1,
            chunks_received: 1, // chunks are merged
            body: String::from("Wikipedia in\r\n\r\nchunks."),
            requests_received: 1,
        });
    }

    // FIXME: Fix parsing newline delimited headers.
    // http://tools.ietf.org/html/rfc7230#section-3.5
    // It is directly supported by httparse and some websites
    // (eg. Github API Errors) and clients use this.
    #[test]
    #[should_panic]
    fn test_newline_delimited() {
        let mut io = MemIo::new();
        let mut lp = MockLoop::new(Default::default());
        io.push_bytes("GET / HTTP/1.1\n\
            Content-Length: 0\n\
            Connection: close\n\n".as_bytes());
        println!("{:?}", io);
        let m = Stream::<Parser<Proto, MemIo>>::accepted(
            io.clone(), &mut lp.scope(1)).expect_machine();
        m.ready(EventSet::readable(), &mut lp.scope(1))
            .expect_machine();
        assert_eq!(*lp.ctx(), Context {
            progressive: false,
            headers_received: 1,
            body: String::from(""),
            chunks_received: 0,
            requests_received: 1,
        });
    }

    #[test]
    #[should_panic]
    fn test_pipelining() {
        let mut io = MemIo::new();
        let mut lp = MockLoop::new(Default::default());
        io.push_bytes("GET /foo HTTP/1.1\r\n\
            Host: example.com\r\n\
            Content-Length: 0\r\n\
            \r\n\
            GET /bar HTTP/1.1\r\n\
            Host: example.com\r\n\
            Content-Length: 0\r\n\
            \r\n".as_bytes());
        println!("{:?}", io);
        let m = Stream::<Parser<Proto, MemIo>>::accepted(
            io.clone(), &mut lp.scope(1)).expect_machine();
        m.ready(EventSet::readable(), &mut lp.scope(1))
            .expect_machine();
        assert_eq!(*lp.ctx(), Context {
            progressive: false,
            headers_received: 2,
            body: String::from(""),
            chunks_received: 0,
            requests_received: 2,
        });
    }

    // FIXME: leading whitespace, causes assertion failure
    // In the interest of robustness, a server that is expecting to receive
    // and parse a request-line SHOULD ignore at least one empty line (CRLF)
    // received prior to the request-line.
    // http://tools.ietf.org/html/rfc7230#section-3.5
    #[test]
    fn test_leading_whitespace() {
        let mut io = MemIo::new();
        let mut lp = MockLoop::new(Default::default());
        io.push_bytes("\r\nGET /foo HTTP/1.1\r\n\
            Host: example.com\r\n\r\n".as_bytes());
        let m = Stream::<Parser<Proto, MemIo>>::accepted(
            io.clone(), &mut lp.scope(1)).expect_machine();
        m.ready(EventSet::readable(), &mut lp.scope(1))
            .expect_machine();
        assert_eq!(*lp.ctx(), Context {
            progressive: false,
            headers_received: 1,
            body: String::from(""),
            chunks_received: 0,
            requests_received: 1,
        });
    }

    // FIXME: causes assertion failure
    #[test]
    #[should_panic]
    fn test_broken_http() {
        let mut io = MemIo::new();
        let mut lp = MockLoop::new(Default::default());
        io.push_bytes("GET / HTTP/1.1\r\n\nHost: host\r\n\r\n".as_bytes());
        let m = Stream::<Parser<Proto, MemIo>>::accepted(
            io.clone(), &mut lp.scope(1)).expect_machine();
        m.ready(EventSet::readable(), &mut lp.scope(1))
            .expect_machine();
    }

    #[test]
    fn test_crazy() {
        let mut io = MemIo::new();
        let mut lp = MockLoop::new(Default::default());
        io.push_bytes("~36!$543&..JKLHfF+Dkjk /foo/$bar HTTP/1.1\r\n\r\n".as_bytes());
        let m = Stream::<Parser<Proto, MemIo>>::accepted(
            io.clone(), &mut lp.scope(1)).expect_machine();
        m.ready(EventSet::readable(), &mut lp.scope(1))
            .expect_machine();
        assert_eq!(*lp.ctx(), Context {
            progressive: false,
            headers_received: 1,
            body: String::from(""),
            chunks_received: 0,
            requests_received: 1,
        });
    }

    #[bench]
    fn bench_parse1(b: &mut Bencher) {
        let mut io = MemIo::new();
        let mut lp = MockLoop::new(Default::default());
        let mut counter = 0;
        b.iter(|| {
            counter += 1;
            let mut m = Stream::<Parser<Proto, MemIo>>::accepted(
                io.clone(), &mut lp.scope(1)).expect_machine();
            io.push_bytes("GET / HTTP/1.1\r\n");
            io.push_bytes("Host: blog.nemo.org\r\n");
            io.push_bytes("User-Agent: Mozilla/5.0 (X11; Linux x86_64");
            io.push_bytes("; rv:44.0) Gecko/20100101 Firefox/44.0\r\n\
            Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8\r\n");
            io.push_bytes("Accept-Language: de-DE,de;q=0.8,en-US;q=0.6,en;q=0.4,fr;q=0.2\r\n\
            Accept-Encoding: gzip, ");
            io.push_bytes("deflate\r\n\
            DNT: 1\r\n\
            Cookie: spam=foo.bar\r\n\
            Connection: keep-alive\r\n\
            If-Modified-Since: Tue, 01 Mar 2016 19:40:42 GMT\r\n");
            io.push_bytes("Cache-Control: max-age=0\r\n\r\n");
            m = m.ready(EventSet::readable(), &mut lp.scope(1)).expect_machine();
            m = m.ready(EventSet::readable(), &mut lp.scope(1)).expect_machine();
            m = m.ready(EventSet::readable(), &mut lp.scope(1)).expect_machine();
            m = m.ready(EventSet::readable(), &mut lp.scope(1)).expect_machine();
            m = m.ready(EventSet::readable(), &mut lp.scope(1)).expect_machine();
            m = m.ready(EventSet::readable(), &mut lp.scope(1)).expect_machine();
            m.ready(EventSet::readable(), &mut lp.scope(1)).expect_machine();
        });
        assert_eq!(*lp.ctx(), Context {
            progressive: false,
            headers_received: counter,
            body: String::from(""),
            chunks_received: 0,
            requests_received: counter,
        });
    }

    #[bench]
    fn bench_parse6(b: &mut Bencher) {
        let mut io = MemIo::new();
        let mut lp = MockLoop::new(Default::default());
        let mut counter = 0;
        b.iter(|| {
            counter += 1;
            let mut m = Stream::<Parser<Proto, MemIo>>::accepted(
                io.clone(), &mut lp.scope(1)).expect_machine();
            io.push_bytes("GET / HTTP/1.1\r\n");
            m = m.ready(EventSet::readable(), &mut lp.scope(1)).expect_machine();
            io.push_bytes("Host: blog.nemo.org\r\n");
            m = m.ready(EventSet::readable(), &mut lp.scope(1)).expect_machine();
            io.push_bytes("User-Agent: Mozilla/5.0 (X11; Linux x86_64");
            m = m.ready(EventSet::readable(), &mut lp.scope(1)).expect_machine();
            io.push_bytes("; rv:44.0) Gecko/20100101 Firefox/44.0\r\n\
            Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8\r\n");
            m = m.ready(EventSet::readable(), &mut lp.scope(1)).expect_machine();
            io.push_bytes("Accept-Language: de-DE,de;q=0.8,en-US;q=0.6,en;q=0.4,fr;q=0.2\r\n\
            Accept-Encoding: gzip, ");
            m = m.ready(EventSet::readable(), &mut lp.scope(1)).expect_machine();
            io.push_bytes("deflate\r\n\
            DNT: 1\r\n\
            Cookie: spam=foo.bar\r\n\
            Connection: keep-alive\r\n\
            If-Modified-Since: Tue, 01 Mar 2016 19:40:42 GMT\r\n");
            m = m.ready(EventSet::readable(), &mut lp.scope(1)).expect_machine();
            io.push_bytes("Cache-Control: max-age=0\r\n\r\n");
            m.ready(EventSet::readable(), &mut lp.scope(1)).expect_machine();
        });
        assert_eq!(*lp.ctx(), Context {
            progressive: false,
            headers_received: counter,
            body: String::from(""),
            chunks_received: 0,
            requests_received: counter,
        });
    }

}
