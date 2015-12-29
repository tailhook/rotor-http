extern crate hyper;
extern crate rotor;
extern crate rotor_stream;
extern crate rotor_http;
extern crate mio;
extern crate time;


use rotor::Scope;
use hyper::status::StatusCode::{self, NotFound};
use hyper::header::ContentLength;
use rotor_stream::{Deadline, Accept, Stream};
use rotor_http::server::{RecvMode, Server, Head, Response, Parser};
use mio::tcp::{TcpListener, TcpStream};
use time::Duration;


struct Context {
    counter: usize,
}

trait Counter {
    fn increment(&mut self);
    fn get(&self) -> usize;
}

impl Counter for Context {
    fn increment(&mut self) { self.counter += 1; }
    fn get(&self) -> usize { self.counter }
}

impl rotor_http::server::Context for Context {
    // default impl is okay
    fn byte_timeout(&self) -> Duration {
        Duration::seconds(1000)
    }
}

enum HelloWorld {
    Start,
    Hello,
    GetNum,
    HelloName(String),
    PageNotFound,
}

fn send_string(res: &mut Response, data: &[u8]) {
    res.status(hyper::status::StatusCode::Ok);
    res.add_header(ContentLength(data.len() as u64)).unwrap();
    res.done_headers().unwrap();
    res.write_body(data);
    res.done();
}

impl<C:Counter+rotor_http::server::Context> Server<C> for HelloWorld {
    fn headers_received(_head: &Head, _scope: &mut Scope<C>)
        -> Result<(Self, RecvMode, Deadline), StatusCode>
    {
        Ok((HelloWorld::Start, RecvMode::Buffered(1024),
            Deadline::now() + Duration::seconds(10)))
    }
    fn request_start(self, head: Head, _res: &mut Response,
        scope: &mut Scope<C>)
        -> Option<Self>
    {
        use self::HelloWorld::*;
        scope.increment();
        match head.uri {
            hyper::uri::RequestUri::AbsolutePath(ref p) if &p[..] == "/" => {
                Some(Hello)
            }
            hyper::uri::RequestUri::AbsolutePath(ref p) if &p[..] == "/num"
            => {
                Some(GetNum)
            }
            hyper::uri::RequestUri::AbsolutePath(p) => {
                Some(HelloName(p[1..].to_string()))
            }
            _ => {
                Some(PageNotFound)
            }
        }
    }
    fn request_received(self, _data: &[u8], res: &mut Response,
        scope: &mut Scope<C>)
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
            Start|PageNotFound => {
                scope.emit_error_page(NotFound, res);
            }
        }
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
        -> Option<Self>
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
    let mut event_loop = mio::EventLoop::new().unwrap();
    let mut handler = rotor::Handler::new(Context {
        counter: 0,
    }, &mut event_loop);
    let lst = TcpListener::bind(&"127.0.0.1:3000".parse().unwrap()).unwrap();
    let ok = handler.add_machine_with(&mut event_loop, |scope| {
        Accept::<TcpListener, TcpStream,
            Stream<Context, _, Parser<HelloWorld>>>::new(lst, scope)
    }).is_ok();
    assert!(ok);
    event_loop.run(&mut handler).unwrap();
}
