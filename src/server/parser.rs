use std::usize;
use std::cmp::min;

use netbuf::MAX_BUF_SIZE;
use rotor::Scope;
use rotor_stream::{Protocol, StreamSocket, Deadline, Expectation as E};
use rotor_stream::{Request, Transport};
use hyper::status::StatusCode::{self, PayloadTooLarge};
use hyper::method::Method::Head;
use hyper::header::Expect;

use super::{MAX_HEADERS_NUM, MAX_HEADERS_SIZE, MAX_CHUNK_HEAD};
use super::{Response};
use super::protocol::{Server, RecvMode};
use super::context::Context;
use super::request::Head;
use super::body::BodyKind;
use super::ResponseImpl;


struct ReadBody<M: Sized> {
    machine: M,
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


pub enum Parser<M: Sized> {
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
        Some((Parser::DoneResponse, E::Flush(0),
              Deadline::now() + scope.byte_timeout()))
    }
}

fn start_headers<C: Context, M: Sized>(scope: &mut Scope<C>)
    -> Request<Parser<M>>
{
    Some((Parser::ReadHeaders,
          E::Delimiter(0, b"\r\n\r\n", MAX_HEADERS_SIZE),
          Deadline::now() + scope.byte_timeout()))
}

fn start_body(mode: RecvMode, body: BodyKind) -> BodyProgress {
    use super::body::BodyKind::*;
    use super::protocol::RecvMode::*;
    use self::BodyProgress::*;

    match (mode, body) {
        // The size of Fixed(x) is checked in parse_headers
        (Buffered(x), Fixed(y)) => BufferFixed(y as usize),
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
    scope: &mut Scope<C>) -> Result<(Head, ReadBody<M>), bool>
    where M: Server<C, S>,
          S: StreamSocket,
          C: Context,
{
    use self::Parser::*;
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
            Ok((head, ReadBody {
                machine: m,
                deadline: dline,
                progress: start_body(mode, body),
                response: Response::new(transport, is_head).internal(),
            }))
        }
        Err(status) => {
            let mut resp = Response::new(transport, is_head);
            scope.emit_error_page(status, &mut resp);
            resp.done();
            Err(can_keep_alive)
        }
    }
}

impl<M> Parser<M>
{
    fn request<C>(self, scope: &mut Scope<C>) -> Request<Parser<M>>
        where C: Context
    {
        use rotor_stream::Expectation::*;
        use self::Parser::*;
        use self::BodyProgress::*;
        let (exp, dline) = match *&self {
            Idle => (Bytes(0), None),
            ReadHeaders => (Delimiter(0, b"\r\n\r\n", MAX_HEADERS_SIZE), None),
            ReadingBody(ref b) => {
                let exp = match *&b.progress {
                    BufferFixed(x) => Bytes(x),
                    BufferEOF(x) => Eof(x),
                    BufferChunked(limit, off, 0)
                    => Delimiter(off, b"\r\n", off+MAX_CHUNK_HEAD),
                    BufferChunked(limit, off, y) => Bytes(y),
                    ProgressiveFixed(hint, left)
                    => Bytes(min(hint as u64, left) as usize),
                    ProgressiveEOF(hint) => Eof(hint),
                    ProgressiveChunked(hint, 0)
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
        Some((self, exp, deadline))
    }
}

impl<C, M, S> Protocol<C, S> for Parser<M>
    where M: Server<C, S>,
          S: StreamSocket,
          C: Context,
{
    type Seed = ();
    fn create(_seed: (), _sock: &mut S, scope: &mut Scope<C>)
        -> Request<Self>
    {
        Some((Parser::Idle, E::Bytes(1),
            Deadline::now() + scope.byte_timeout()))
    }
    fn bytes_read(self, transport: &mut Transport<S>,
                  end: usize, scope: &mut Scope<C>)
        -> Request<Self>
    {
        use self::Parser::*;
        match self {
            Idle => {
                start_headers(scope)
            }
            ReadHeaders => {
                match parse_headers::<C, M, S>(transport, end, scope) {
                    Ok((head, body)) => {
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
            me @ ReadingBody(_) => {
                unimplemented!();
            }
            // Spurious event?
            me @ DoneResponse => me.request(scope),
            Processing(m, r, dline) => Some((Processing(m, r, dline),
                                             E::Sleep, dline)),
        }
    }
    fn bytes_flushed(self, _transport: &mut Transport<S>,
                     scope: &mut Scope<C>)
        -> Request<Self>
    {
        match self {
            Parser::DoneResponse => None,
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
        use self::Parser::*;
        use self::BodyProgress::*;
        // We may just flush and exit in every state. But:
        // 1. The match asserts that we know which state parser may be in
        // 2. We may send more specific response, so that browser will not
        //    retry ugly request multiple times
        match self {
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
    fn wakeup(self, transport: &mut Transport<S>, scope: &mut Scope<C>)
        -> Request<Self>
    {
        use self::Parser::*;
        match self {
            me@Idle | me@ReadHeaders | me@DoneResponse => me.request(scope),
            ReadingBody(reader) => {
                unimplemented!();
            }
            Processing(..) => {
                unimplemented!();
            }
        }
    }
}
