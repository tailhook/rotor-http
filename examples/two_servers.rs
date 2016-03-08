extern crate rotor;
extern crate rotor_http;

use std::time::Duration;

use rotor::{Scope, Compose2, Time};
use rotor_http::server::{Fsm, RecvMode, Server, Head, Response};
use rotor::mio::tcp::{TcpListener};


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
    res.status(200, "OK");
    res.add_length(data.len() as u64).unwrap();
    res.done_headers().unwrap();
    res.write_body(data);
    res.done();
}

impl Server for Incr {
    type Context = Context;
    fn headers_received(_head: Head, _res: &mut Response,
        scope: &mut Scope<Context>)
        -> Option<(Self, RecvMode, Time)>
    {
        scope.increment();
        Some((Incr, RecvMode::Buffered(1024),
            scope.now() + Duration::new(10, 0)))
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
        -> Option<(Self, Time)>
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
    fn headers_received(_head: Head, _res: &mut Response,
        scope: &mut Scope<Context>)
        -> Option<(Self, RecvMode, Time)>
    {
        Some((Get, RecvMode::Buffered(1024),
            scope.now() + Duration::new(10, 0)))
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
        -> Option<(Self, Time)>
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
    let event_loop = rotor::Loop::new(&rotor::Config::new()).unwrap();
    let mut loop_inst = event_loop.instantiate(Context {
        counter: 0,
    });
    loop_inst.add_machine_with(|scope| {
        Fsm::<Incr, _>::new(lst1, scope).wrap(Compose2::A)
    }).unwrap();
    loop_inst.add_machine_with(|scope| {
        Fsm::<Get, _>::new(lst2, scope).wrap(Compose2::B)
    }).unwrap();
    loop_inst.run().unwrap();
}
