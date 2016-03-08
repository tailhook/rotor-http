extern crate rotor;
extern crate rotor_http;

use std::io::stdout;
use std::io::Write;
use std::net::ToSocketAddrs;
use std::time::Duration;

use rotor::{Scope, Time};
use rotor_http::client::{connect_tcp, Request, Head, Client, RecvMode,
    Connection, Requester, Task, Version};

struct Context;

struct Cli(Option<String>);
struct Req(String);

impl Client for Cli {
    type Requester = Req;
    type Seed = String;
    fn create(seed: Self::Seed,
        _scope: &mut Scope<<Self::Requester as Requester>::Context>)
        -> Self
    {
        Cli(Some(seed))
    }
    fn connection_idle(mut self, _conn: &Connection,
        scope: &mut Scope<Context>)
        -> Task<Cli>
    {
        match self.0.take() {
            Some(req) => Task::Request(Cli(None), Req(req)),
            None => {
                scope.shutdown_loop();
                Task::Close
            }
        }
    }
    fn wakeup(self,
        _connection: &Connection,
        _scope: &mut Scope<<Self::Requester as Requester>::Context>)
        -> Task<Cli>
    {
        unimplemented!();
    }
    fn timeout(self,
        _connection: &Connection,
        _scope: &mut Scope<<Self::Requester as Requester>::Context>)
        -> Task<Cli>
    {
        unimplemented!();
    }
}

impl Requester for Req {
    type Context = Context;
    fn prepare_request(self, req: &mut Request) -> Option<Self> {
        req.start("GET", &self.0, Version::Http11);
        req.done_headers().unwrap();
        req.done();
        Some(self)
    }
    fn headers_received(self, head: Head, _request: &mut Request,
        scope: &mut Scope<Self::Context>)
        -> Option<(Self, RecvMode, Time)>
    {
        println!("----- Headers -----");
        println!("Status: {} {}", head.code, head.reason);
        for header in head.headers {
            println!("{}: {}", header.name,
                String::from_utf8_lossy(header.value));
        }
        Some((self,  RecvMode::Buffered(16386),
            scope.now() + Duration::new(1000, 0)))
    }
    fn response_received(self, data: &[u8], _request: &mut Request,
        _scope: &mut Scope<Self::Context>)
    {
        println!("----- Response -----");
        stdout().write_all(data).unwrap();
        if data.last() != Some(&b'\n') {
            println!("");
        }
    }
    fn response_chunk(self, _chunk: &[u8], _request: &mut Request,
        _scope: &mut Scope<Self::Context>)
        -> Option<Self>
    {
        unreachable!();
    }
    fn response_end(self, _request: &mut Request,
        _scope: &mut Scope<Self::Context>)
    {
        unreachable!();
    }
    fn timeout(self, _request: &mut Request, _scope: &mut Scope<Self::Context>)
        -> Option<(Self, Time)>
    {
        unreachable!();
    }
    fn wakeup(self, _request: &mut Request, _scope: &mut Scope<Self::Context>)
        -> Option<Self>
    {
        unimplemented!();
    }
}


fn main() {
    let creator = rotor::Loop::new(&rotor::Config::new()).unwrap();
    let mut loop_inst = creator.instantiate(Context);
    loop_inst.add_machine_with(|scope| {
        connect_tcp::<Cli>(scope,
            &("google.com", 80).to_socket_addrs()
                .unwrap().collect::<Vec<_>>()[0],
            format!("/"))
    }).unwrap();
    loop_inst.run().unwrap();
}
