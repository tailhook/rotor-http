extern crate rotor;
extern crate rotor_stream;
extern crate rotor_http;
extern crate time;


use rotor::{Scope, Compose2};
use rotor_http::status::StatusCode::{self};
use rotor_http::header::ContentLength;
use rotor_stream::{Deadline, Accept, Stream};
use rotor_http::server::{RecvMode, Server, Head, Response, Parser};
use rotor::mio::tcp::{TcpListener, TcpStream};
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
    res.status(StatusCode::Ok);
    res.add_header(ContentLength(data.len() as u64)).unwrap();
    res.done_headers().unwrap();
    res.write_body(data);
    res.done();
}

impl Server for Incr {
    type Context = Context;
    fn headers_received(_head: &Head, _scope: &mut Scope<Context>)
        -> Result<(Self, RecvMode, Deadline), StatusCode>
    {
        Ok((Incr, RecvMode::Buffered(1024),
            Deadline::now() + Duration::seconds(10)))
    }
    fn request_start(self, _head: Head, _res: &mut Response,
        scope: &mut Scope<Context>)
        -> Option<Self>
    {
        scope.increment();
        Some(self)
    }
    fn request_received(self, _data: &[u8], res: &mut Response,
        _scope: &mut Scope<Context>)
        -> Option<Self>
    {
        send_string(res, b"Hello World!");
        None
    }
    fn request_chunk(self, _chunk: &[u8], _response: &mut Response,
        _scope: &mut Scope<Context>)
        -> Option<Self>
    {
        unreachable!();
    }

    /// End of request body, only for Progressive requests
    fn request_end(self, _response: &mut Response, _scope: &mut Scope<Context>)
        -> Option<Self>
    {
        unreachable!();
    }

    fn timeout(self, _response: &mut Response, _scope: &mut Scope<Context>)
        -> Option<(Self, Deadline)>
    {
        unimplemented!();
    }
    fn wakeup(self, _response: &mut Response, _scope: &mut Scope<Context>)
        -> Option<Self>
    {
        unimplemented!();
    }
}

impl Server for Get {
    type Context = Context;
    fn headers_received(_head: &Head, _scope: &mut Scope<Context>)
        -> Result<(Self, RecvMode, Deadline), StatusCode>
    {
        Ok((Get, RecvMode::Buffered(1024),
            Deadline::now() + Duration::seconds(10)))
    }
    fn request_start(self, _head: Head, _res: &mut Response,
        _scope: &mut Scope<Context>)
        -> Option<Self>
    {
        Some(self)
    }
    fn request_received(self, _data: &[u8], res: &mut Response,
        scope: &mut Scope<Context>)
        -> Option<Self>
    {
        send_string(res,
            format!("This host has been visited {} times",
                scope.get())
            .as_bytes());
        None
    }
    fn request_chunk(self, _chunk: &[u8], _response: &mut Response,
        _scope: &mut Scope<Context>)
        -> Option<Self>
    {
        unreachable!();
    }

    /// End of request body, only for Progressive requests
    fn request_end(self, _response: &mut Response, _scope: &mut Scope<Context>)
        -> Option<Self>
    {
        unreachable!();
    }

    fn timeout(self, _response: &mut Response, _scope: &mut Scope<Context>)
        -> Option<(Self, Deadline)>
    {
        unimplemented!();
    }
    fn wakeup(self, _response: &mut Response, _scope: &mut Scope<Context>)
        -> Option<Self>
    {
        unimplemented!();
    }
}

fn main() {
    let lst1 = TcpListener::bind(&"127.0.0.1:3000".parse().unwrap()).unwrap();
    let lst2 = TcpListener::bind(&"127.0.0.1:3001".parse().unwrap()).unwrap();
    let mut event_loop = rotor::EventLoop::new().unwrap();
    let mut handler = rotor::Handler::new(Context {
        counter: 0,
    }, &mut event_loop);
    let ok1 = handler.add_machine_with(&mut event_loop, |scope| {
        Accept::<Stream<Parser<Incr, _>>, _>::new(lst1, scope).map(Compose2::A)
    }).is_ok();
    let ok2 = handler.add_machine_with(&mut event_loop, |scope| {
        Accept::<Stream<Parser<Get, _>>, _>::new(lst2, scope).map(Compose2::B)
    }).is_ok();
    assert!(ok1 && ok2);
    event_loop.run(&mut handler).unwrap();
}
