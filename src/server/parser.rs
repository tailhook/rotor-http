use std::any::Any;
use std::cmp::min;
use std::marker::PhantomData;
use std::str::from_utf8;

use httparse::{EMPTY_HEADER, Request, parse_chunk_size};
use rotor::{Scope, Time};
use rotor::mio::tcp::TcpStream;
use rotor_stream::{Exception, Intent, Protocol, StreamSocket, Transport};

use version::Version;
use headers;
use message::MessageState;
use recvmode::RecvMode;
use super::{MAX_HEADERS_NUM, MAX_HEADERS_SIZE, MAX_CHUNK_HEAD, Context, Head, Response, Server};
use super::body::BodyKind;
use super::response::state;
use super::error::RequestError;

#[derive(Debug)]
pub struct ReadBody<M: Server> {
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

fn scan_raw_request(raw_request: &Request)
    -> Result<(BodyKind, bool, bool, bool), RequestError>
{
    // Implements the body length algorithm for requests:
    // http://httpwg.github.io/specs/rfc7230.html#message.body.length
    //
    // The length of a request body is determined by one of the following
    // (in order of precedence):
    //
    // 1. If the request contains a valid `Transfer-Encoding` header
    //    with `chunked` as the last encoding the request is chunked
    //    (3rd option in RFC).
    // 2. If the request contains a valid `Content-Length` header
    //    the request has the given length in octets
    //    (5th option in RFC).
    // 3. If neither `Transfer-Encoding` nor `Content-Length` are
    //    present the request has an empty body
    //    (6th option in RFC).
    // 4. In all other cases the request is a bad request.
    use super::body::BodyKind::*;
    use super::RequestError::*;
    let is_head = raw_request.method.unwrap() == "HEAD";
    let mut has_content_length = false;
    let mut close = raw_request.version.unwrap() == 0;
    let mut expect_continue = false;
    let mut body = Fixed(0);
    for header in raw_request.headers.iter() {
        if headers::is_transfer_encoding(header.name) {
            if let Some(enc) = header.value.split(|&x| x == b',').last() {
                if headers::is_chunked(enc) {
                    if has_content_length {
                        // override but don't allow keep-alive
                        close = true;
                    }
                    body = Chunked;
                }
            }
        } else if headers::is_content_length(header.name) {
            if has_content_length {
                // duplicate content_length
                return Err(DuplicateContentLength);
            }
            has_content_length = true;
            if body != Chunked {
                let s = try!(from_utf8(header.value));
                let len = try!(s.parse().map_err(BadContentLength));
                body = Fixed(len);
            } else {
                // transfer-encoding has preference and don't allow keep-alive
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
    Ok((body, is_head, expect_continue, close))
}

#[inline]
fn consumed(off: usize) -> usize {
    // If buffer is not empty it has final '\r\n' at the
    // end, and we are going to search for the next pair
    // But the `off` is stored as a number of useful bytes in the buffer
    if off > 0 { off+2 } else { 0 }
}

#[derive(Debug)]
pub enum ParserImpl<M: Server> {
    Idle,
    ReadHeaders,
    ReadingBody(ReadBody<M>),
    Processing(M, MessageState, bool, Time),
    DoneResponse,
}

impl <M: Server>ParserImpl<M> {
    fn wrap<S: StreamSocket>(self) -> Parser<M, S> {
        Parser(self, PhantomData)
    }
}

#[derive(Debug)]
pub struct Parser<M, S>(ParserImpl<M>, PhantomData<*const S>)
    where M: Server, S: StreamSocket;

unsafe impl<M, S> Send for Parser<M, S>
    where M: Server+Send, S: StreamSocket
{}

unsafe impl<M, S> Sync for Parser<M, S>
    where M: Server+Sync, S: StreamSocket
{}


impl<M: Server, S: StreamSocket> Parser<M, S> {
    #[inline]
    fn intent_idle(scope: &Scope<M::Context>) -> Intent<Self> {
        Intent::of(ParserImpl::Idle.wrap())
            .expect_bytes(1)
            .deadline(scope.now() + scope.byte_timeout())
    }
    #[inline]
    fn intent_headers(scope: &Scope<M::Context>, n: usize) -> Intent<Self> {
        Intent::of(ParserImpl::ReadHeaders.wrap())
            .expect_bytes(n + 1)
            .deadline(scope.now() + scope.byte_timeout())
    }
    #[inline]
    fn intent_flush(scope: &Scope<M::Context>) -> Intent<Self> {
        Intent::of(ParserImpl::DoneResponse.wrap())
            .expect_flush()
            .deadline(scope.now() + scope.byte_timeout())
    }
    fn intent_body(body: ReadBody<M>) -> Intent<Self> {
        use rotor_stream::Expectation::*;
        use self::BodyProgress::*;
        let exp = match *&body.progress {
            BufferFixed(x) => Bytes(x),
            BufferChunked(_, off, 0) => {
                Delimiter(consumed(off), b"\r\n", consumed(off) + MAX_CHUNK_HEAD)
            }
            BufferChunked(_, off, y) => Bytes(off + y + 2),
            ProgressiveFixed(hint, left) => Bytes(min(hint as u64, left) as usize),
            ProgressiveChunked(_, off, 0) => Delimiter(off, b"\r\n", off + MAX_CHUNK_HEAD),
            ProgressiveChunked(hint, off, left) => {
                Bytes(min(hint as u64, off as u64 + left) as usize + 2)
            }
        };
        let deadline = body.deadline;
        Intent::of(ParserImpl::ReadingBody(body).wrap()).expect(exp).deadline(deadline)
    }
    fn complete<'x>(scope: &mut Scope<M::Context>,
                    machine: Option<M>,
                    response: Response<'x>,
                    connection_close: bool,
                    deadline: Time)
                    -> Intent<Parser<M, S>> {
        match machine {
            Some(m) => {
                Intent::of(ParserImpl::Processing(m, state(response), connection_close, deadline).wrap())
                    .sleep()
                    .deadline(deadline)
            }
            None => {
                // TODO(tailhook) probably we should do something better than
                // an assert?
                assert!(response.is_complete());
                if connection_close {
                    Parser::intent_flush(scope)
                } else {
                    Parser::intent_idle(scope)
                }
            }
        }
    }
}

impl<M: Server, S: StreamSocket> Protocol for Parser<M, S> {
    type Context = M::Context;
    type Socket = S;
    type Seed = ();
    fn create(_seed: Self::Seed,
              _sock: &mut Self::Socket,
              scope: &mut Scope<Self::Context>)
              -> Intent<Self> {
        Parser::intent_idle(scope)
    }
    fn bytes_read(self,
                  transport: &mut Transport<Self::Socket>,
                  end: usize,
                  scope: &mut Scope<Self::Context>)
                  -> Intent<Self> {
        use self::ParserImpl::*;
        use super::RequestError::*;
        match self.0 {
            Idle | ReadHeaders => {
                use httparse::Status::*;
                let n;
                let client = Any::downcast_ref::<TcpStream>(transport.socket())
                                 .and_then(|x| x.peer_addr().ok());
                let (input, output) = transport.buffers();
                let ((machine, mode, deadline), response, body, close) = {
                    let mut headers = [EMPTY_HEADER; MAX_HEADERS_NUM];
                    let mut raw_request = Request::new(&mut headers);
                    n = match raw_request.parse(&input[..]) {
                        Ok(Complete(n)) => n,
                        Ok(Partial) => {
                            if input.len() > MAX_HEADERS_SIZE {
                                let mut response = Response::new(output,
                                                                 Version::Http10,
                                                                 false,
                                                                 true);
                                scope.emit_error_page(&HeadersAreTooLarge,
                                                      &mut response);
                                return Parser::intent_flush(scope);
                            }
                            return Parser::intent_headers(scope, input.len());
                        }
                        Err(e) => {
                            let mut response = Response::new(output,
                                Version::Http10, false, true);
                            scope.emit_error_page(&RequestError::from(e),
                                                  &mut response);
                            return Parser::intent_flush(scope);
                        }
                    };
                    match scan_raw_request(&raw_request) {
                        Ok((body, is_head, expect_continue, close)) => {
                            let version = if raw_request.version.unwrap() == 1 {
                                Version::Http11
                            } else {
                                Version::Http10
                            };
                            let request = Head {
                                client: client,
                                version: version,
                                method: raw_request.method.unwrap(),
                                scheme: "http",
                                path: raw_request.path.unwrap(),
                                headers: raw_request.headers,
                                body_kind: body,
                            };
                            let mut response = Response::new(output,
                                request.version, is_head, close);
                            let triple = M::headers_received(request,
                                &mut response, scope);
                            if triple.is_none() && response.is_started() {
                                if !expect_continue {
                                    return Intent::done();
                                } else {
                                    return Parser::intent_flush(scope);
                                }
                            } else if triple.is_none() {
                                scope.emit_error_page(&HeadersReceived,
                                    &mut response);
                                return Parser::intent_flush(scope);
                            }
                            if expect_continue {
                                response.response_continue();
                            }
                            (triple.unwrap(), response, body, close)
                        }
                        Err(e) => {
                            let mut response = Response::new(output,
                                Version::Http10, false, true);
                            scope.emit_error_page(&e, &mut response);
                            return Parser::intent_flush(scope);
                        }
                    }
                };
                input.consume(n);
                return Parser::intent_body(ReadBody {
                    machine: Some(machine),
                    deadline: deadline,
                    progress: start_body(mode, body),
                    response: state(response),
                    connection_close: close,
                });
            }
            ReadingBody(rb) => {
                use self::BodyProgress::*;
                let (inp, out) = transport.buffers();
                let mut resp = rb.response.with(out);
                let (m, progress) = match rb.progress {
                    BufferFixed(x) => {
                        let m = rb.machine
                                  .and_then(|m| m.request_received(&inp[..x], &mut resp, scope));
                        inp.consume(x);
                        (m, None)
                    }
                    BufferChunked(limit, off, 0) => {
                        use httparse::Status::*;
                        let lenstart = consumed(off);
                        match parse_chunk_size(&inp[lenstart..lenstart + end + 2]) {
                            Ok(Complete((_, 0))) => {
                                inp.remove_range(off..lenstart + end + 2);
                                let m = rb.machine.and_then(|m| {
                                    m.request_received(&inp[..off], &mut resp, scope)
                                });
                                inp.consume(off);
                                (m, None)
                            }
                            Ok(Complete((_, chunk_len))) => {
                                if off as u64 + chunk_len > limit as u64 {
                                    inp.consume(lenstart + end + 2);
                                    rb.machine.map(|m| m.bad_request(&mut resp, scope));
                                    scope.emit_error_page(&PayloadTooLarge,
                                        &mut resp);
                                    return Parser::intent_flush(scope);
                                }
                                inp.remove_range(off..lenstart + end + 2);
                                (rb.machine,
                                 Some(BufferChunked(limit, off, chunk_len as usize)))
                            }
                            Ok(Partial) => unreachable!(),
                            Err(e) => {
                                inp.consume(lenstart + end + 2);
                                rb.machine.map(|m| m.bad_request(&mut resp, scope));
                                scope.emit_error_page(&RequestError::from(e),
                                                      &mut resp);
                                return Parser::intent_flush(scope);
                            }
                        }
                    }
                    BufferChunked(limit, off, bytes) => {
                        debug_assert_eq!(off + bytes, end - 2);
                        // We keep final \r\n in the buffer, so we can cut
                        // it together with next chunk length
                        // (i.e. do not do `remove_range` twice)
                        (rb.machine, Some(BufferChunked(limit, off + bytes, 0)))
                    }
                    ProgressiveFixed(hint, mut left) => {
                        let real_bytes = min(inp.len() as u64, left) as usize;
                        let m = rb.machine.and_then(|m| {
                            m.request_chunk(&inp[..real_bytes], &mut resp, scope)
                        });
                        inp.consume(real_bytes);
                        left -= real_bytes as u64;
                        if left == 0 {
                            let m = m.and_then(|m| m.request_end(&mut resp, scope));
                            (m, None)
                        } else {
                            (m, Some(ProgressiveFixed(hint, left)))
                        }
                    }
                    ProgressiveChunked(hint, off, 0) => {
                        use httparse::Status::*;
                        match parse_chunk_size(&inp[off..off + end + 2]) {
                            Ok(Complete((_, 0))) => {
                                inp.remove_range(off..off + end + 2);
                                let mut m = rb.machine;
                                if off > 0 {
                                    m = m.and_then(|m| {
                                        m.request_chunk(&inp[..off], &mut resp, scope)
                                    });
                                }
                                m = m.and_then(|m| m.request_end(&mut resp, scope));
                                inp.consume(off);
                                (m, None)
                            }
                            Ok(Complete((_, chunk_len))) => {
                                inp.remove_range(off..off + end + 2);
                                (rb.machine, Some(ProgressiveChunked(hint, off, chunk_len)))
                            }
                            Ok(Partial) => unreachable!(),
                            Err(e) => {
                                inp.consume(off + end + 2);
                                rb.machine.map(|m| m.bad_request(&mut resp, scope));
                                scope.emit_error_page(&RequestError::from(e),
                                                      &mut resp);
                                return Parser::intent_flush(scope);
                            }
                        }
                    }
                    ProgressiveChunked(hint, off, mut left) => {
                        let ln = if off as u64 + left == (end - 2) as u64 {
                            // in progressive chunked we remove final '\r\n'
                            // immediately to make code simpler
                            // may be optimized later
                            inp.remove_range(end - 2..end);
                            off + left as usize
                        } else {
                            inp.len()
                        };
                        left -= (ln - off) as u64;
                        if ln < hint {
                            (rb.machine, Some(ProgressiveChunked(hint, ln, left)))
                        } else {
                            let m = rb.machine
                                      .and_then(|m| m.request_chunk(&inp[..ln], &mut resp, scope));
                            inp.consume(ln);
                            (m, Some(ProgressiveChunked(hint, 0, left)))
                        }
                    }
                };
                match progress {
                    Some(p) => {
                        Parser::intent_body(ReadBody {
                            machine: m,
                            deadline: rb.deadline,
                            progress: p,
                            response: state(resp),
                            connection_close: rb.connection_close,
                        })
                    }
                    None => Parser::complete(scope, m, resp, rb.connection_close, rb.deadline),
                }
            }
            Processing(m, r, c, dline) => {
                Intent::of(Processing(m, r, c, dline).wrap())
                    .sleep().deadline(dline)
            },
            /// TODO(tailhook) fix output timeout
            DoneResponse => Parser::intent_flush(scope),
        }
    }
    fn bytes_flushed(self,
                     _transport: &mut Transport<Self::Socket>,
                     _scope: &mut Scope<Self::Context>)
                     -> Intent<Self> {
        match self.0 {
            ParserImpl::DoneResponse => Intent::done(),
            _ => unreachable!(),
        }
    }
    fn timeout(self,
               transport: &mut Transport<Self::Socket>,
               scope: &mut Scope<Self::Context>)
               -> Intent<Self> {
        use self::ParserImpl::*;
        use super::RequestError::*;
        match self.0 {
            Idle | DoneResponse => Intent::done(),
            ReadHeaders => {
                let output = transport.output();
                let mut response = Response::new(output,
                    Version::Http10, false, true);
                scope.emit_error_page(&HeadersTimeout, &mut response);
                Parser::intent_flush(scope)
            }
            ReadingBody(rb) => {
                let mut resp = rb.response.with(transport.output());
                let res = rb.machine.and_then(|m| m.timeout(&mut resp, scope));
                match res {
                    Some((m, deadline)) => {
                        Parser::intent_body(ReadBody {
                            machine: Some(m),
                            deadline: deadline,
                            progress: rb.progress,
                            response: state(resp),
                            connection_close: rb.connection_close,
                        })
                    }
                    None => {
                        if !resp.is_started() {
                            scope.emit_error_page(&RequestTimeout, &mut resp);
                            Parser::intent_flush(scope)
                        } else {
                            Intent::done()
                        }
                    }
                }
            }
            Processing(m, respimp, close, _) => {
                let mut resp = respimp.with(transport.output());
                match m.timeout(&mut resp, scope) {
                    Some((m, dline)) => Parser::complete(scope, Some(m), resp, close, dline),
                    None => {
                        if !resp.is_started() {
                            scope.emit_error_page(&HandlerTimeout, &mut resp);
                            Parser::intent_flush(scope)
                        } else {
                            Intent::done()
                        }
                    }
                }
            }
        }
    }
    fn wakeup(self,
              transport: &mut Transport<Self::Socket>,
              scope: &mut Scope<Self::Context>)
              -> Intent<Self> {
        use self::ParserImpl::*;
        match self.0 {
            Idle => Parser::intent_idle(scope),
            ReadHeaders => Parser::intent_headers(scope, transport.input().len()),
            DoneResponse => Parser::intent_flush(scope),
            ReadingBody(rb) => {
                let mut resp = rb.response.with(transport.output());
                let m = rb.machine.and_then(|m| m.wakeup(&mut resp, scope));
                Parser::intent_body(ReadBody {
                    machine: m,
                    deadline: rb.deadline,
                    progress: rb.progress,
                    response: state(resp),
                    connection_close: rb.connection_close,
                })
            }
            Processing(m, respimp, close, dline) => {
                let mut resp = respimp.with(transport.output());
                let mres = m.wakeup(&mut resp, scope);
                Parser::complete(scope, mres, resp, close, dline)
            }
        }
    }

    fn exception(self,
                 transport: &mut Transport<Self::Socket>,
                 reason: Exception,
                 scope: &mut Scope<Self::Context>)
                 -> Intent<Self> {
        use rotor_stream::Exception::*;
        use self::BodyProgress::*;
        use self::ParserImpl::*;
        use super::error::RequestError::*;
        match reason {
            LimitReached => {
                if let ReadingBody(rb) = self.0 {
                    assert!(matches!(rb.progress,
                        ProgressiveChunked(_, _, 0) |  // TODO(tailhook) why?
                        BufferChunked(_, _, 0)));
                    let mut resp = rb.response.with(transport.output());
                    rb.machine.map(|m| m.bad_request(&mut resp, scope));
                    if !resp.is_started() {
                        scope.emit_error_page(&PayloadTooLarge, &mut resp);
                    }
                    if resp.is_complete() {
                        return Parser::intent_flush(scope)
                    }
                }
            }
            EndOfStream => {
                if let ReadingBody(rb) = self.0 {
                    let mut resp = rb.response.with(transport.output());
                    rb.machine.map(|m| m.bad_request(&mut resp, scope));
                    if !resp.is_started() {
                        scope.emit_error_page(&PrematureEndOfStream,
                                              &mut resp);
                    }
                    if resp.is_complete() {
                        return Parser::intent_flush(scope);
                    }
                }
            }
            _ => (),
        }
        Intent::done()
    }
}

#[cfg(test)]
mod test {
    #[cfg(feature="nightly")]
    use test::Bencher;
    use std::default::Default;
    use std::time::Duration;
    use std::str::from_utf8;
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

    #[test]
    fn test_newline_delimited() {
        let mut io = MemIo::new();
        let mut lp = MockLoop::new(Default::default());
        io.push_bytes("GET / HTTP/1.1\n\
            Content-Length: 0\n\
            Connection: close\n\n"
                          .as_bytes());
        let m = Stream::<Parser<Proto, MemIo>>::accepted(io.clone(), &mut lp.scope(1))
                    .expect_machine();
        m.ready(EventSet::readable(), &mut lp.scope(1))
         .expect_machine();
        assert_eq!(*lp.ctx(),
                   Context {
                       progressive: false,
                       headers_received: 1,
                       body: String::from(""),
                       chunks_received: 0,
                       requests_received: 1,
                   });
    }

    #[test]
    fn test_leading_whitespace() {
        let mut io = MemIo::new();
        let mut lp = MockLoop::new(Default::default());
        io.push_bytes("\r\nGET /foo HTTP/1.1\r\n\
            Host: example.com\r\n\r\n"
                          .as_bytes());
        let m = Stream::<Parser<Proto, MemIo>>::accepted(io.clone(), &mut lp.scope(1))
                    .expect_machine();
        m.ready(EventSet::readable(), &mut lp.scope(1))
         .expect_machine();
        assert_eq!(*lp.ctx(),
                   Context {
                       progressive: false,
                       headers_received: 1,
                       body: String::from(""),
                       chunks_received: 0,
                       requests_received: 1,
                   });
    }

    #[test]
    fn test_crazy() {
        let mut io = MemIo::new();
        let mut lp = MockLoop::new(Default::default());
        io.push_bytes("~36!$543&..JKLHfF+Dkjk /foo/$bar HTTP/1.1\r\n\r\n".as_bytes());
        let m = Stream::<Parser<Proto, MemIo>>::accepted(io.clone(), &mut lp.scope(1))
                    .expect_machine();
        m.ready(EventSet::readable(), &mut lp.scope(1))
         .expect_machine();
        assert_eq!(*lp.ctx(),
                   Context {
                       progressive: false,
                       headers_received: 1,
                       body: String::from(""),
                       chunks_received: 0,
                       requests_received: 1,
                   });
    }
    #[cfg(feature="nightly")]
    #[bench]
    fn bench_parse1(b: &mut Bencher) {
        let mut io = MemIo::new();
        let mut lp = MockLoop::new(Default::default());
        let mut counter = 0;
        b.iter(|| {
            counter += 1;
            let mut m = Stream::<Parser<Proto, MemIo>>::accepted(io.clone(), &mut lp.scope(1))
                            .expect_machine();
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
        assert_eq!(*lp.ctx(),
                   Context {
                       progressive: false,
                       headers_received: counter,
                       body: String::from(""),
                       chunks_received: 0,
                       requests_received: counter,
                   });
    }
    #[cfg(feature="nightly")]
    #[bench]
    fn bench_parse6(b: &mut Bencher) {
        let mut io = MemIo::new();
        let mut lp = MockLoop::new(Default::default());
        let mut counter = 0;
        b.iter(|| {
            counter += 1;
            let mut m = Stream::<Parser<Proto, MemIo>>::accepted(io.clone(), &mut lp.scope(1))
                            .expect_machine();
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
        assert_eq!(*lp.ctx(),
                   Context {
                       progressive: false,
                       headers_received: counter,
                       body: String::from(""),
                       chunks_received: 0,
                       requests_received: counter,
                   });
    }
}
