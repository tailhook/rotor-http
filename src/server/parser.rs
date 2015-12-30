use std::cmp::min;
use std::str::from_utf8;

use netbuf::MAX_BUF_SIZE;
use rotor::Scope;
use rotor_stream::{Protocol, StreamSocket, Deadline, Expectation as E};
use rotor_stream::{Request, Transport};
use hyper::status::StatusCode::{PayloadTooLarge, BadRequest};
use hyper::method::Method::Head;
use hyper::header::Expect;

use super::{MAX_HEADERS_SIZE, MAX_CHUNK_HEAD};
use super::{Response};
use super::protocol::{Server, RecvMode};
use super::context::Context;
use super::request::Head;
use super::body::BodyKind;
use super::ResponseImpl;


struct ReadBody<M: Sized> {
    machine: Option<M>,
    deadline: Deadline,
    progress: BodyProgress,
    response: ResponseImpl,
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
    /// Progressive with chunked encoding (hint, bytes left for current chunk)
    // To be able to merge chunks, should add offset here
    ProgressiveChunked(usize, u64),
}

pub struct Parser<M: Sized>(ParserImpl<M>);

enum ParserImpl<M: Sized> {
    Idle,
    ReadHeaders,
    ReadingBody(ReadBody<M>),
    /// Close connection after buffer is flushed. In other cases -> Idle
    Processing(M, ResponseImpl, Deadline),
    DoneResponse,
}

impl<M> Parser<M>
{
    fn flush<C>(scope: &mut Scope<C>) -> Request<Parser<M>>
        where C: Context
    {
        Some((Parser(ParserImpl::DoneResponse), E::Flush(0),
              Deadline::now() + scope.byte_timeout()))
    }
    fn bad_request<'x, C>(scope: &mut Scope<C>, mut response: Response<'x>)
        -> Request<Parser<M>>
        where C: Context
    {
        if !response.is_started() {
            scope.emit_error_page(BadRequest, &mut response);
        }
        response.finish();
        Some((Parser(ParserImpl::DoneResponse), E::Flush(0),
              Deadline::now() + scope.byte_timeout()))
    }
}

fn start_headers<C: Context, M: Sized>(scope: &mut Scope<C>)
    -> Request<Parser<M>>
{
    Some((Parser(ParserImpl::ReadHeaders),
          E::Delimiter(0, b"\r\n\r\n", MAX_HEADERS_SIZE),
          Deadline::now() + scope.byte_timeout()))
}

fn start_body(mode: RecvMode, body: BodyKind) -> BodyProgress {
    use super::body::BodyKind::*;
    use super::protocol::RecvMode::*;
    use self::BodyProgress::*;

    match (mode, body) {
        // The size of Fixed(x) is checked in parse_headers
        (Buffered(_), Fixed(y)) => BufferFixed(y as usize),
        (Buffered(x), Chunked) => BufferChunked(x, 0, 0),
        (Buffered(x), Eof) => BufferEOF(x),
        (Progressive(x), Fixed(y)) => ProgressiveFixed(x, y),
        (Progressive(x), Chunked) => ProgressiveChunked(x, 0),
        (Progressive(x), Eof) => ProgressiveEOF(x),
        (_, Upgrade) => unimplemented!(),
    }
}

// Parses headers
//
// On error returns bool, which is true if keep-alive connection can be
// carried on.
fn parse_headers<C, M, S>(transport: &mut Transport<S>, end: usize,
    scope: &mut Scope<C>) -> Result<ReadBody<M>, bool>
    where M: Server<C>,
          S: StreamSocket,
          C: Context,
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
            let mut resp = Response::new(transport.output(), &head);
            Ok(ReadBody {
                machine: m.request_start(head, &mut resp, scope),
                deadline: dline,
                progress: start_body(mode, body),
                response: resp.internal(),
            })
        }
        Err(status) => {
            let mut resp = Response::simple(transport.output(), is_head);
            scope.emit_error_page(status, &mut resp);
            let okay = resp.finish();
            Err(can_keep_alive && okay)
        }
    }
}

impl<M> ParserImpl<M>
{
    fn request<C>(self, scope: &mut Scope<C>) -> Request<Parser<M>>
        where C: Context
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
                    BufferEOF(x) => BufferEof(x),
                    BufferChunked(_, off, 0)
                    => Delimiter(off, b"\r\n", off+MAX_CHUNK_HEAD),
                    BufferChunked(_, off, y) => Bytes(off + y),
                    ProgressiveFixed(hint, left)
                    => Bytes(min(hint as u64, left) as usize),
                    ProgressiveEOF(hint) => Eof(hint),
                    ProgressiveChunked(_, 0)
                    => Delimiter(0, b"\r\n", 0+MAX_CHUNK_HEAD),
                    ProgressiveChunked(hint, left)
                    => Bytes(min(hint as u64, left) as usize)
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
        Some((Parser(self), exp, deadline))
    }
}

impl<C, M, S> Protocol<C, S> for Parser<M>
    where M: Server<C>,
          S: StreamSocket,
          C: Context,
{
    type Seed = ();
    fn create(_seed: (), _sock: &mut S, scope: &mut Scope<C>)
        -> Request<Self>
    {
        Some((Parser(ParserImpl::Idle), E::Bytes(1),
            Deadline::now() + scope.byte_timeout()))
    }
    fn bytes_read(self, transport: &mut Transport<S>,
                  end: usize, scope: &mut Scope<C>)
        -> Request<Self>
    {
        use self::ParserImpl::*;
        use self::BodyProgress::*;
        match self.0 {
            Idle => {
                start_headers(scope)
            }
            ReadHeaders => {
                match parse_headers::<C, M, S>(transport, end, scope) {
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
                    BufferEOF(_) => {
                        let len = inp.len();
                        let m = rb.machine.and_then(
                            |m| m.request_received(
                                            &inp[..len], &mut resp, scope));
                        inp.consume(len);
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
                                inp.remove_range(off..);
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
                                    return Parser::bad_request(scope, resp);
                                }
                                (rb.machine,
                                    Some(BufferChunked(limit, off,
                                                  chunk_len as usize)))
                            }
                            None => {
                                inp.consume(end+2);
                                rb.machine.map(
                                    |m| m.bad_request(&mut resp, scope));
                                return Parser::bad_request(scope, resp);
                            }
                        }
                    }
                    BufferChunked(limit, off, _) => {
                        (rb.machine, Some(BufferChunked(limit, off, 0)))
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
                    ProgressiveEOF(_hint) => {
                        // TODO(tailhook) probably stream.eof should be impl
                        unimplemented!();
                    }
                    ProgressiveChunked(_hints, _left) => unimplemented!(),
                };
                match progress {
                    Some(p) => {
                        ReadingBody(ReadBody {
                            machine: m,
                            deadline: rb.deadline,
                            progress: p,
                            response: resp.internal(),
                        }).request(scope)
                    }
                    None => match m {
                        Some(m) => {
                            Some((Parser(Processing(
                                        m, resp.internal(), rb.deadline)),
                                 E::Sleep, rb.deadline))
                        }
                        None => {
                            assert!(resp.is_complete());
                            Idle.request(scope)
                        }
                    }
                }
            }
            // Spurious event?
            me @ DoneResponse => me.request(scope),
            Processing(m, r, dline) => Some((Parser(Processing(m, r, dline)),
                                             E::Sleep, dline)),
        }
    }
    fn bytes_flushed(self, _transport: &mut Transport<S>,
                     scope: &mut Scope<C>)
        -> Request<Self>
    {
        match self.0 {
            ParserImpl::DoneResponse => None,
            me => me.request(scope),
        }
    }
    fn timeout(self, _transport: &mut Transport<S>,
        _scope: &mut Scope<C>)
        -> Request<Self>
    {
        unimplemented!();
    }
    fn delimiter_not_found(self, _transport: &mut Transport<S>,
        scope: &mut Scope<C>)
        -> Request<Self>
    {
        use self::ParserImpl::*;
        use self::BodyProgress::*;
        // We may just flush and exit in every state. But:
        // 1. The match asserts that we know which state parser may be in
        // 2. We may send more specific response, so that browser will not
        //    retry ugly request multiple times
        match self.0 {
            ReadHeaders
                // TODO(tailhook) send RequestHeaderFieldsTooLarge
            | ReadingBody( ReadBody { progress: ProgressiveChunked(_, 0), ..})
            | ReadingBody( ReadBody { progress: BufferChunked(_, _, 0), ..})
                // TODO(tailhook) send BadRequest ?
            => {
                // Should we flush or just close?
                // Probably closing is useful because previous responses might
                // be absolutely valid, and we've got invalid pipelined
                // request
                Parser::flush(scope)
            }
            // TODO(tailhook) Any other weird cases?
            _ => unreachable!(),
        }
    }
    fn wakeup(self, _transport: &mut Transport<S>, scope: &mut Scope<C>)
        -> Request<Self>
    {
        use self::ParserImpl::*;
        match self.0 {
            me@Idle | me@ReadHeaders | me@DoneResponse => me.request(scope),
            ReadingBody(_reader) => {
                unimplemented!();
            }
            Processing(..) => {
                unimplemented!();
            }
        }
    }
}
