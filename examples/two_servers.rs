extern crate hyper;
extern crate rotor;
extern crate rotor_stream;
extern crate rotor_http;
extern crate mio;
extern crate time;


use rotor::{Scope, Compose2};
use hyper::status::StatusCode::{self};
use hyper::header::ContentLength;
use rotor_stream::{Deadline, Accept, Stream};
use rotor_http::server::{RecvMode, Server, Head, Response, Parser};
use mio::tcp::{TcpListener, TcpStream};
use time::Duration;


pub struct Context {
    counter: usize,
}

trait IncrCounter {
    fn increment(&mut self);
}

trait GetCounter {
    fn get(&self) -> usize;
}

impl IncrCounter for Context {
    fn increment(&mut self) { self.counter += 1; }
}

impl GetCounter for Context {
    fn get(&self) -> usize { self.counter }
}

struct Incr;

struct Get;

impl rotor_http::server::Context for Context {
    // default impl is okay
}

fn send_string(res: &mut Response, data: &[u8]) {
    res.status(hyper::status::StatusCode::Ok);
    res.add_header(ContentLength(data.len() as u64)).unwrap();
    res.done_headers().unwrap();
    res.write_body(data);
    res.done();
}

impl<C:IncrCounter+rotor_http::server::Context> Server<C> for Incr {
    fn headers_received(_head: &Head, _scope: &mut Scope<C>)
        -> Result<(Self, RecvMode, Deadline), StatusCode>
    {
        Ok((Incr, RecvMode::Buffered(1024),
            Deadline::now() + Duration::seconds(10)))
    }
    fn request_start(self, _head: Head, _res: &mut Response,
        scope: &mut Scope<C>)
        -> Option<Self>
    {
        scope.increment();
        Some(self)
    }
    fn request_received(self, _data: &[u8], res: &mut Response,
        _scope: &mut Scope<C>)
        -> Option<Self>
    {
        send_string(res, b"Hello World!");
        None
    }
    fn request_chunk(self, _chunk: &[u8], _response: &mut Response,
        _scope: &mut Scope<C>)
        -> Option<Self>
    {
        unreachable!();
    }

    /// End of request body, only for Progressive requests
    fn request_end(self, _response: &mut Response, _scope: &mut Scope<C>)
        -> Option<Self>
    {
        unreachable!();
    }

    fn timeout(self, _response: &mut Response, _scope: &mut Scope<C>)
        -> Option<(Self, Deadline)>
    {
        unimplemented!();
    }
    fn wakeup(self, _response: &mut Response, _scope: &mut Scope<C>)
        -> Option<Self>
    {
        unimplemented!();
    }
}

impl<C:GetCounter+rotor_http::server::Context> Server<C> for Get {
    fn headers_received(_head: &Head, _scope: &mut Scope<C>)
        -> Result<(Self, RecvMode, Deadline), StatusCode>
    {
        Ok((Get, RecvMode::Buffered(1024),
            Deadline::now() + Duration::seconds(10)))
    }
    fn request_start(self, _head: Head, _res: &mut Response,
        _scope: &mut Scope<C>)
        -> Option<Self>
    {
        Some(self)
    }
    fn request_received(self, _data: &[u8], res: &mut Response,
        scope: &mut Scope<C>)
        -> Option<Self>
    {
        send_string(res,
            format!("This host has been visited {} times",
                scope.get())
            .as_bytes());
        None
    }
    fn request_chunk(self, _chunk: &[u8], _response: &mut Response,
        _scope: &mut Scope<C>)
        -> Option<Self>
    {
        unreachable!();
    }

    /// End of request body, only for Progressive requests
    fn request_end(self, _response: &mut Response, _scope: &mut Scope<C>)
        -> Option<Self>
    {
        unreachable!();
    }

    fn timeout(self, _response: &mut Response, _scope: &mut Scope<C>)
        -> Option<(Self, Deadline)>
    {
        unimplemented!();
    }
    fn wakeup(self, _response: &mut Response, _scope: &mut Scope<C>)
        -> Option<Self>
    {
        unimplemented!();
    }
}

fn main() {
    let lst1 = TcpListener::bind(&"127.0.0.1:3000".parse().unwrap()).unwrap();
    let lst2 = TcpListener::bind(&"127.0.0.1:3001".parse().unwrap()).unwrap();
    let mut event_loop = mio::EventLoop::new().unwrap();
    let mut handler = rotor::Handler::new(Context {
        counter: 0,
    }, &mut event_loop);
    let ok1 = handler.add_machine_with(&mut event_loop, |scope| {
        Accept::<TcpListener, TcpStream,
            Stream<Context, _, Parser<Incr>>>::new(
                lst1, scope)
            .map(Compose2::A)
    }).is_ok();
    let ok2 = handler.add_machine_with(&mut event_loop, |scope| {
        Accept::<TcpListener, TcpStream,
            Stream<Context, _, Parser<Get>>>::new(
                lst2, scope)
            .map(Compose2::B)
    }).is_ok();
    assert!(ok1 && ok2);
    event_loop.run(&mut handler).unwrap();
}
