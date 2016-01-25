use std::cmp::min;
use std::str::from_utf8;
use std::marker::PhantomData;

use rotor_stream::MAX_BUF_SIZE;
use rotor::Scope;
use rotor_stream::{Protocol, StreamSocket, Deadline, Expectation as E};
use rotor_stream::{Request as Task, Transport, Exception};
use hyper::status::StatusCode::{PayloadTooLarge, BadRequest, RequestTimeout};
use hyper::status::StatusCode::{self, RequestHeaderFieldsTooLarge};
use hyper::method::Method::Head;
use hyper::header::Expect;

use recvmode::RecvMode;
use message::{MessageState};
use super::{MAX_HEADERS_SIZE, MAX_CHUNK_HEAD};
use super::request::Request;
use super::protocol::{Client};
use super::head::Head;
use super::body::BodyKind;
use super::request::state;


struct ReadBody<M: Client> {
    machine: Option<M>,
    deadline: Deadline,
    progress: BodyProgress,
    response: MessageState,
}

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
    Idle,
    ReadHeaders,
    ReadingBody(ReadBody<M>),
    /// Close connection after buffer is flushed. In other cases -> Idle
    Processing(M, MessageState, Deadline),
    DoneResponse,
}

impl<M: Client, S: StreamSocket> Parser<M, S> {
    fn flush(scope: &mut Scope<M::Context>) -> Task<Parser<M, S>>
    {
        Some((ParserImpl::DoneResponse.wrap(), E::Flush(0),
              Deadline::now() + M::byte_timeout()))
    }
    fn error<'x>(scope: &mut Scope<M::Context>, mut response: Request<'x>,
        code: StatusCode)
        -> Task<Parser<M, S>>
    {
        if !response.is_started() {
            scope.emit_error_page(code, &mut response);
        }
        response.finish();
        Some((ParserImpl::DoneResponse.wrap(), E::Flush(0),
              Deadline::now() + M::byte_timeout()))
    }
    fn raw_error<'x>(scope: &mut Scope<M::Context>,
        transport: &mut Transport<<Self as Protocol>::Socket>,
        code: StatusCode)
        -> Task<Parser<M, S>>
    {
        let resp = Request::simple(transport.output(), false);
        Parser::error(scope, resp, code)
    }
    fn complete<'x, C>(scope: &mut Scope<C>, machine: Option<M>,
        response: Request<'x>, deadline: Deadline)
        -> Task<Parser<M, S>>
    {
        match machine {
            Some(m) => {
                Some((ParserImpl::Processing(m, state(response), deadline)
                      .wrap(),
                    E::Sleep, deadline))
            }
            None => {
                // TODO(tailhook) probably we should do something better than
                // an assert?
                assert!(response.is_complete());
                ParserImpl::Idle.request(scope)
            }
        }
    }
}

fn start_headers<C, M: Client, S: StreamSocket>(scope: &mut Scope<C>)
    -> Task<Parser<M, S>>
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
        (Buffered(x), Eof) => BufferEOF(x),
        (Progressive(x), Fixed(y)) => ProgressiveFixed(x, y),
        (Progressive(x), Chunked) => ProgressiveChunked(x, 0, 0),
        (Progressive(x), Eof) => ProgressiveEOF(x),
        (_, Upgrade) => unimplemented!(),
    }
}

// Parses headers
//
// On error returns bool, which is true if keep-alive connection can be
// carried on.
fn parse_headers<S, M>(transport: &mut Transport<S>, end: usize,
    scope: &mut Scope<M::Context>) -> Result<ReadBody<M>, bool>
    where M: Client, S: StreamSocket,
{
    // Determines if we can keep-alive after error response.
    // We may not be able to keep keep-alive for multiple reasons:
    //
    // 1. When request headers are too wrong
    //    (probably client connects with wrong protocol)
    //
    // 2. When request contains non-empty request body (we don't
    //    want to wait until it is uploaded just to send error)
    //
    // Note we definitely can't keep alive if we can't say
    // whether request method is HEAD
    //
    // All of these are important to avoid cache poisoning attacks
    // on proxy servers.
    let mut can_keep_alive = false;
    // Determines if we can safely send the response body
    let mut is_head = false;

    let status = match Head::parse(&transport.input()[..end+4]) {
        Ok(head) => {
            is_head = head.method == Head;
            match M::headers_received(&head, scope) {
                Ok((_, RecvMode::Buffered(x), _)) if x >= MAX_BUF_SIZE
                => panic!("Can't buffer {} bytes, max {}",
                          x, MAX_BUF_SIZE),
                Ok((m, mode, dline)) => {
                    match BodyKind::parse(&head) {
                        Ok(body) => {
                            // TODO(tailhook)
                            // Probably can handle small
                            // request bodies that are already
                            // in the buffer
                            if body == BodyKind::Fixed(0) {
                                can_keep_alive = true;
                            }
                            match (body, mode) {
                                (BodyKind::Fixed(x), RecvMode::Buffered(y))
                                if x >= y as u64 => {
                                    Err(PayloadTooLarge)
                                }
                                _ => {
                                    Ok((head, body, m, mode, dline))
                                }
                            }
                        }
                        Err(status) => Err(status),
                    }
                }
                Err(status) => Err(status),
            }
        }
        Err(status) => Err(status),
    };
    transport.input().consume(end+4);
    match status {
        Ok((head, body, m, mode, dline)) => {
            if head.headers.get::<Expect>() == Some(&Expect::Continue) {
                // Handler has already approved request, so just push it
                transport.output().extend(
                    format!("{} 100 Continue\r\n\r\n", head.version)
                    .as_bytes());
            }
            let mut resp = Request::new(transport.output(), &head);
            Ok(ReadBody {
                machine: m.request_start(head, &mut resp, scope),
                deadline: dline,
                progress: start_body(mode, body),
                response: state(resp),
            })
        }
        Err(status) => {
            let mut resp = Request::simple(transport.output(), is_head);
            scope.emit_error_page(status, &mut resp);
            let okay = resp.finish();
            Err(can_keep_alive && okay)
        }
    }
}

impl<M: Client> ParserImpl<M>
{
    fn wrap<S: StreamSocket>(self) -> Parser<M, S>
    {
        Parser(self, PhantomData)
    }
    fn request<C, S>(self, scope: &mut Scope<C>) -> Task<Parser<M, S>>
        where S: StreamSocket
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
                    BufferEOF(x) => Bytes(x),
                    BufferChunked(_, off, 0)
                    => Delimiter(off, b"\r\n", off+MAX_CHUNK_HEAD),
                    BufferChunked(_, off, y) => Bytes(off + y),
                    ProgressiveFixed(hint, left)
                    => Bytes(min(hint as u64, left) as usize),
                    ProgressiveEOF(hint) => Bytes(hint),
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

impl<S: StreamSocket, M: Client> Protocol for Parser<M, S> {
    type Context = M::Context;
    type Socket = S;
    type Seed = ();
    fn create(_seed: (), _sock: &mut Self::Socket,
        scope: &mut Scope<M::Context>)
        -> Task<Self>
    {
        Some((ParserImpl::Idle.wrap(), E::Bytes(1),
            Deadline::now() + scope.byte_timeout()))
    }
    fn bytes_read(self, transport: &mut Transport<S>,
                  end: usize, scope: &mut Scope<M::Context>)
        -> Task<Self>
    {
        use self::ParserImpl::*;
        use self::BodyProgress::*;
        match self.0 {
            Idle => {
                start_headers(scope)
            }
            ReadHeaders => {
                match parse_headers(transport, end, scope) {
                    Ok(body) => {
                        ReadingBody(body).request(scope)
                    }
                    Err(can_keep_alive) => {
                        if can_keep_alive {
                            Idle.request(scope)
                        } else {
                            Parser::flush(scope)
                        }
                    }
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
                    BufferEOF(_) => unreachable!(),
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
                    ProgressiveEOF(hint) => {
                        let ln = inp.len();
                        let m = rb.machine.and_then(
                            |m| m.request_chunk(&inp[..ln], &mut resp, scope));
                        (m, Some(ProgressiveEOF(hint)))
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
                        }).request(scope)
                    }
                    None => Parser::complete(scope, m, resp, rb.deadline)
                }
            }
            // Spurious event?
            me @ DoneResponse => me.request(scope),
            Processing(m, r, dline) => Some((Processing(m, r, dline).wrap(),
                                             E::Sleep, dline)),
        }
    }
    fn bytes_flushed(self, _transport: &mut Transport<S>,
                     scope: &mut Scope<Self::Context>)
        -> Task<Self>
    {
        match self.0 {
            ParserImpl::DoneResponse => None,
            me => me.request(scope),
        }
    }
    fn exception(self, transport: &mut Transport<S>, exc: Exception,
        scope: &mut Scope<Self::Context>)
        -> Task<Self>
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
                        match rb.progress {
                            BufferEOF(_) | ProgressiveEOF(_) => {
                                let (inp, out) = transport.buffers();
                                let mut resp = rb.response.with(out);
                                let mut m = rb.machine;
                                if inp.len() > 0 {
                                    m = m.and_then(
                                        |m| m.request_chunk(
                                            &inp[..], &mut resp, scope));
                                }
                                m = m.and_then(
                                    |m| m.request_end(&mut resp, scope));
                                Parser::complete(scope, m, resp, rb.deadline)
                            }
                            _ => {
                                // Incomplete request
                                let mut resp = rb.response.with(
                                    transport.output());
                                rb.machine.map(
                                    |m| m.bad_request(&mut resp, scope));
                                Parser::error(scope, resp, BadRequest)
                            }
                        }
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
        -> Task<Self>
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
                        }).request(scope)
                    }
                    None => Parser::error(scope, resp, RequestTimeout),
                }
            }
            Processing(m, respimp, _) => {
                let mut resp = respimp.with(transport.output());
                match m.timeout(&mut resp, scope) {
                    Some((m, dline)) => {
                        Parser::complete(scope, Some(m), resp, dline)
                    }
                    None => Parser::error(scope, resp, RequestTimeout),
                }
            }
        }
    }
    fn wakeup(self, transport: &mut Transport<S>,
        scope: &mut Scope<Self::Context>)
        -> Task<Self>
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
                }).request(scope)
            }
            Processing(m, respimp, dline) => {
                let mut resp = respimp.with(transport.output());
                let mres = m.wakeup(&mut resp, scope);
                Parser::complete(scope, mres, resp, dline)
            }
        }
    }
}
