use std::usize;
use std::cmp::min;

use netbuf;
use rotor::Scope;
use rotor_stream::{Protocol, StreamSocket, Deadline, Expectation as E};
use rotor_stream::{Request, Transport};
use hyper::status::StatusCode::{self, PayloadTooLarge};
use hyper::method::Method::Head;
use hyper::header::Expect;

use super::{MAX_HEADERS_NUM, MAX_HEADERS_SIZE};
use super::{Response};
use super::protocol::{Server, RecvMode};
use super::context::Context;
use super::request::Head;
use super::body::BodyKind;


struct ReadBody<M: Sized> {
    machine: M,
    deadline: Deadline,
    progress: BodyProgress,
}

pub enum BodyProgress {
    /// Buffered fixed-size request (bytes left)
    BufferFixed(usize),
    /// Buffered request till end of input
    BufferEOF,
    /// Buffered request with chunked encoding (bytes left for current chunk)
    BufferChunked(usize),
    /// Progressive fixed-size request (size hint, bytes left)
    ProgressiveFixed(usize, u64),
    /// Progressive till end of input (size hint)
    ProgressiveEOF(usize),
    /// Progressive with chunked encoding (hint, bytes left for current chunk)
    ProgressiveChunked(usize, u64),
}


pub enum Parser<M: Sized> {
    Idle,
    ReadHeaders,
    ReadingBody(ReadBody<M>),
    /// Close connection after buffer is flushed. In other cases -> Idle
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
          E::Delimiter(b"\r\n\r\n", MAX_HEADERS_SIZE),
          Deadline::now() + scope.byte_timeout()))
}

fn start_body(mode: RecvMode, body: BodyKind) -> BodyProgress {
    use super::body::BodyKind::*;
    use super::protocol::RecvMode::*;
    use self::BodyProgress::*;

    match (mode, body) {
        // The size of Fixed(x) is checked above
        (Buffered, Fixed(x)) => BufferFixed(x as usize),
        (Buffered, Chunked) => BufferChunked(0),
        (Buffered, Eof) => BufferEOF,
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

    // TODO(tailhook) this should be contant
    let MAX_BUFFERED_BODY: u64 = min(usize::MAX as u64,
                                     netbuf::MAX_BUF_SIZE as u64);

    let status = match Head::parse(&transport.input()[..end+4]) {
        Ok(head) => {
            is_head = head.method == Head;
            match M::headers_received(&head, scope) {
                Ok((m, mode, dline)) => {
                    match BodyKind::parse(&head) {
                        // Note this check is needed for avoiding overflow
                        // later
                        Ok(BodyKind::Fixed(x)) if x > MAX_BUFFERED_BODY
                        => Err(PayloadTooLarge),
                        Ok(body) => {
                            // TODO(tailhook)
                            // Probably can handle small
                            // request bodies that are already
                            // in the buffer
                            if body == BodyKind::Fixed(0) {
                                can_keep_alive = true;
                            }
                            Ok((head, body, m, mode, dline))
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
            Ok(ReadBody {
                machine: m,
                deadline: dline,
                progress: start_body(mode, body)
            })
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
            ReadHeaders => (Delimiter(b"\r\n\r\n", MAX_HEADERS_SIZE), None),
            ReadingBody(ref b) => {
                let exp = match *&b.progress {
                    BufferFixed(x) => Bytes(x),
                    // No such stuff in Stream
                    BufferEOF => unimplemented!(),
                    BufferChunked(0) => {
                        unimplemented!();
                    }
                    BufferChunked(x) => {
                        // Need "Delimiter starting from" abstraction
                        unimplemented!();
                    }
                    ProgressiveFixed(hint, left) => {
                        Bytes(min(hint as u64, left) as usize)
                    }
                    // No such stuff in Stream
                    ProgressiveEOF(hint) => {
                        unimplemented!();
                    }
                    ProgressiveChunked(hint, 0) => {
                        unimplemented!();
                    }
                    ProgressiveChunked(hint, left) => {
                        Bytes(min(hint as u64, left) as usize)
                    }
                };
                (exp, Some(b.deadline))
            }
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
                    Ok(body) => {
                        ReadingBody(body).request(scope)
                    }
                    Err(can_keep_alive) => {
                        if can_keep_alive {
                            start_headers(scope)
                        } else {
                            Parser::flush(scope)
                        }
                    }
                }
            }
            me @ ReadingBody(_) => me.request(scope),
            me @ DoneResponse => me.request(scope),
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
        match self {
            Idle => None,
            ReadHeaders => {
                // TODO(tailhook) send RequestHeaderFieldsTooLarge
                Parser::flush(scope)
            }
            _ => unimplemented!(),
        }
    }
    fn wakeup(self, transport: &mut Transport<S>, _scope: &mut Scope<C>)
        -> Request<Self>
    {
        unimplemented!();
    }
}
