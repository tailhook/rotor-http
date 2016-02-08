extern crate rotor;
extern crate rotor_http;
extern crate rotor_tools;
extern crate time;

use std::net::ToSocketAddrs;

use rotor::{Scope};
use rotor_http::client::{Pool, create_pool, Request, Head, Client, RecvMode};
use rotor_http::method::Method::Get;
use rotor_http::version::HttpVersion::Http11;
use rotor_http::Deadline;

struct Context {
    pool: Pool<Req>,
}

struct Req(String);

impl Client for Req {
    type Context = Context;
    fn prepare_request(self, req: &mut Request) -> Option<Self> {
        req.start(Get, &self.0, Http11);
        req.done_headers().unwrap();
        req.done();
        Some(self)
    }
    fn headers_received(self, head: &Head, request: &mut Request,
        scope: &mut Scope<Self::Context>)
        -> Option<(Self, RecvMode, Deadline)>
    {
        Some((self,  RecvMode::Buffered(16386), Deadline::now() +
            time::Duration::seconds(1000)))
    }
    fn response_received(self, data: &[u8], request: &mut Request,
        scope: &mut Scope<Self::Context>)
    {
        unreachable!();
    }
    fn response_start(self, head: Head, request: &mut Request,
        scope: &mut Scope<Self::Context>)
        -> Option<Self>
    {
        unreachable!();
    }
    fn response_chunk(self, chunk: &[u8], request: &mut Request,
        scope: &mut Scope<Self::Context>)
        -> Option<Self>
    {
        unreachable!();
    }
    fn response_end(self, request: &mut Request,
        scope: &mut Scope<Self::Context>)
    {
        unreachable!();
    }
    fn timeout(self, request: &mut Request, scope: &mut Scope<Self::Context>)
        -> Option<(Self, Deadline)>
    {
        unreachable!();
    }
    fn wakeup(self, request: &mut Request, scope: &mut Scope<Self::Context>)
        -> Option<Self>
    {
        unimplemented!();
    }
}


fn main() {
    let mut creator = rotor::Loop::new(&rotor::Config::new()).unwrap();
    let pool = creator.add_and_fetch(|scope| {
        create_pool(scope)
    }).unwrap();
    pool.request_to(
        ("google.com", 80).to_socket_addrs().unwrap().collect::<Vec<_>>()[0],
        Req(format!("/")));
    let loop_inst = creator.instantiate(Context {
        pool: pool,
    });
    //let lst = TcpListener::bind(&"127.0.0.1:3000".parse().unwrap()).unwrap();
    //loop_inst.add_machine_with(|scope| {
    //    Accept::<Stream<Parser<HelloWorld, _>>, _>::new(lst, scope)
    //}).unwrap();
    loop_inst.run().unwrap();
}
