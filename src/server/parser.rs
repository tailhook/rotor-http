use rotor::Scope;
use rotor_stream::{Protocol, StreamSocket, Deadline, Expectation as E};
use rotor_stream::{Request, Transport};
use hyper::status::StatusCode;
use hyper::method::Method::Head;

use super::{MAX_HEADERS_NUM, MAX_HEADERS_SIZE};
use super::{Response};
use super::protocol::{Server, RecvMode};
use super::context::Context;
use super::request::Head;
use super::body::BodyKind;


pub enum Parser<M: Sized> {
    Idle,
    ReadHeaders,
    /// Buffered fixed-size request (bytes left)
    BufferFixed(M, usize),
    /// Buffered request till end of input
    BufferEOF(M),
    /// Buffered request with chunked encoding (bytes left for current chunk)
    BufferChunked(M, usize),
    /// Progressive fixed-size request (bytes left)
    ProgressiveFixed(M, u64),
    /// Progressive till end of input
    ProgressiveEOF(M),
    /// Progressive with chunked encoding (bytes left for current chunk)
    ProgressiveChunked(M, u64),
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

fn parse_headers<C, M, S>(transport: &mut Transport<S>, end: usize,
    scope: &mut Scope<C>) -> Request<Parser<M>>
    where M: Server<C, S>,
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
            // TODO(tailhook) first check for 101 expect, but not for http1.0
            unimplemented!();
        }
        Err(status) => {
            let mut resp = Response::new(transport, is_head);
            scope.emit_error_page(status, &mut resp);
            resp.done();
            return Parser::flush(scope);
        }
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
                Some((Parser::ReadHeaders,
                      E::Delimiter(b"\r\n\r\n", MAX_HEADERS_SIZE),
                      Deadline::now() + scope.byte_timeout()))
            }
            ReadHeaders => {
                parse_headers(transport, end, scope)
            }
            _ => unimplemented!(),
        }
    }
    fn bytes_flushed(self, _transport: &mut Transport<S>,
                     _scope: &mut Scope<C>)
        -> Request<Self>
    {
        unimplemented!();
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
