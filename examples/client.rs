extern crate rotor;
extern crate rotor_http;
extern crate time;

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
    let mut new_pool = None;
    creator.add_machine_with(|scope| {
        let (fsm, pool) = create_pool(scope);
        new_pool = Some(pool);
        Ok(fsm)
    }).unwrap();
    let loop_inst = creator.instantiate(Context {
        pool: new_pool.unwrap(),
    });
    //pool.request(("google.com", 80), Req(format!("/")));
    //let lst = TcpListener::bind(&"127.0.0.1:3000".parse().unwrap()).unwrap();
    //loop_inst.add_machine_with(|scope| {
    //    Accept::<Stream<Parser<HelloWorld, _>>, _>::new(lst, scope)
    //}).unwrap();
    loop_inst.run().unwrap();
}
