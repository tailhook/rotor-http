use rotor::Scope;
use rotor_stream::{Protocol, StreamSocket, Deadline, Expectation as E};
use rotor_stream::{Request, Transport};
use hyper::status::StatusCode;

use super::{MAX_HEADERS_NUM, MAX_HEADERS_SIZE};
use super::protocol::Server;
use super::context::Context;
use super::request::Head;


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
    ProgressiveFixed(M, usize),
    /// Progressive till end of input
    ProgressiveEOF(M),
    /// Progressive with chunked encoding (bytes left for current chunk)
    ProgressiveChunked(M, usize),
    DoneResponse,
}

impl<M> Parser<M>
{
    fn flush<C, S>(self, scope: &mut Scope<C>) -> Request<Parser<M>>
        where S: StreamSocket,
              C: Context,
    {
        Some((Parser::DoneResponse, E::Flush(0),
              Deadline::now() + scope.byte_timeout()))
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
                let inp = transport.input();
                let head = match Head::parse(&inp[..end+4]) {
                    Ok(head) => {
                        unimplemented!();
                    }
                    Err(_) => {
                        unimplemented!();
                        //Some(StatusCode::BadRequest)
                    }
                };
                inp.consume(end+4);
                unimplemented!();
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
            ReadHeaders => self.flush::<_, S>(scope),
            _ => unimplemented!(),
        }
    }
    fn wakeup(self, transport: &mut Transport<S>, _scope: &mut Scope<C>)
        -> Request<Self>
    {
        unimplemented!();
    }
}
