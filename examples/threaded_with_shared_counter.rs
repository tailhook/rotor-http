// same as threaded.rs, except that the counter is thread safe
// and report the correct number of requests, parallel or not

extern crate rotor;
extern crate rotor_http;


use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use rotor::{Scope, Time};
use rotor_http::server::{Fsm, RecvMode, Server, Head, Response};
use rotor::mio::tcp::TcpListener;

struct Context {
    counter: Arc<AtomicUsize>,
}

trait Counter {
    fn increment(&mut self);
    fn get(&self) -> usize;
}

impl Counter for Context {
    fn increment(&mut self) { self.counter.fetch_add(1, Ordering::SeqCst); }
    fn get(&self) -> usize { self.counter.load(Ordering::SeqCst) }
}

impl rotor_http::server::Context for Context {
    // default impl is okay
    fn byte_timeout(&self) -> Duration {
        Duration::new(1000, 0)
    }
}


#[derive(Debug, Clone)]
enum HelloWorld {
    Hello,
    GetNum,
    HelloName(String),
    PageNotFound,
}

fn send_string(res: &mut Response, data: &[u8]) {
    res.status(200, "OK");
    res.add_length(data.len() as u64).unwrap();
    res.done_headers().unwrap();
    res.write_body(data);
    res.done();
}

impl Server for HelloWorld {
    type Context = Context;
    fn headers_received(head: Head, _res: &mut Response,
        scope: &mut Scope<Context>)
        -> Option<(Self, RecvMode, Time)>
    {
        use self::HelloWorld::*;
        scope.increment();
        Some((match head.path {
            "/" => Hello,
            "/num" => GetNum,
            p if p.starts_with('/') => HelloName(p[1..].to_string()),
            _ => PageNotFound
        }, RecvMode::Buffered(1024), scope.now() + Duration::new(10, 0)))
    }
    fn request_received(self, _data: &[u8], res: &mut Response,
        scope: &mut Scope<Context>)
        -> Option<Self>
    {
        use self::HelloWorld::*;
        match self {
            Hello => {
                send_string(res, b"Hello World!");
            }
            GetNum => {
                send_string(res,
                    format!("This host has been visited {} times",
                        scope.get())
                    .as_bytes());
            }
            HelloName(name) => {
                send_string(res, format!("Hello {}!", name).as_bytes());
            }
            PageNotFound => {
                let data = b"404 - Page not found";
                res.status(404, "Not Found");
                res.add_length(data.len() as u64).unwrap();
                res.done_headers().unwrap();
                res.write_body(data);
                res.done();
            }
        }
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
    let lst = TcpListener::bind(&"127.0.0.1:3000".parse().unwrap()).unwrap();
    let threads = env::var("THREADS").unwrap_or("2".to_string())
        .parse().unwrap();
    let mut children = Vec::new();
    let counter = Arc::new(AtomicUsize::new(0));
    for _ in 0..threads {
        let counter_ref = counter.clone();
        let listener = lst.try_clone().unwrap();
        children.push(thread::spawn(move || {
            let event_loop = rotor::Loop::new(
                &rotor::Config::new()).unwrap();
            let mut loop_inst = event_loop.instantiate(Context {
                counter: counter_ref,
            });
            loop_inst.add_machine_with(|scope| {
                Fsm::<HelloWorld, _>::new(listener, scope)
            }).unwrap();
            loop_inst.run().unwrap();
        }));
    }
    for child in children {
        child.join().unwrap();
    }
}
