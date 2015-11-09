extern crate hyper;
extern crate rotor_http;
extern crate rotor;
extern crate mio;

use rotor_http::HttpServer;


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


impl<C:IncrCounter> rotor_http::http1::Handler<C> for Incr {
    fn request(req: rotor_http::http1::Request,
               res: &mut rotor_http::http1::ResponseBuilder,
               ctx: &mut C)
    {
        ctx.increment();
        match req.uri {
            hyper::uri::RequestUri::AbsolutePath(ref p) if &p[..] == "/" => {
                res.set_status(hyper::status::StatusCode::Ok);
                res.put_body("Hello World!");
            }
            _ => {}  // Do nothing: not found or bad request
        }
    }
}

impl<C:GetCounter> rotor_http::http1::Handler<C> for Get {
    fn request(req: rotor_http::http1::Request,
               res: &mut rotor_http::http1::ResponseBuilder,
               ctx: &mut C)
    {
        match req.uri {
            hyper::uri::RequestUri::AbsolutePath(ref p) if &p[..] == "/num"
            => {
                res.set_status(hyper::status::StatusCode::Ok);
                res.put_body(format!("The other port visited {} times",
                                     ctx.get()));
            }
            _ => {}  // Do nothing: not found or bad request
        }
    }
}

enum Wrapper<C:IncrCounter + GetCounter> {
    Incr(HttpServer<C, Incr>),
    Get(HttpServer<C, Get>),
}

impl<C:IncrCounter + GetCounter> rotor::EventMachine<C> for Wrapper<C> {
    /// Socket readiness notification
    fn ready(self, events: mio::EventSet, context: &mut C)
        -> rotor::Async<Self, Option<Self>>
    {
        match self {
            Wrapper::Incr(m) => m.ready(events, context).wrap(Wrapper::Incr),
            Wrapper::Get(m) => m.ready(events, context).wrap(Wrapper::Get),
        }
    }

    /// Gives socket a chance to register in event loop
    fn register(self, reg: &mut rotor::handler::Registrator)
        -> rotor::Async<Self, ()>
    {
        match self {
            Wrapper::Incr(m) => m.register(reg).map(Wrapper::Incr),
            Wrapper::Get(m) => m.register(reg).map(Wrapper::Get),
        }
    }

    /// Timeout happened
    fn timeout(self, context: &mut C)
        -> rotor::Async<Self, Option<Self>>
    {
        match self {
            Wrapper::Incr(m) => m.timeout(context).wrap(Wrapper::Incr),
            Wrapper::Get(m) => m.timeout(context).wrap(Wrapper::Get),
        }
    }

    /// Message received
    fn wakeup(self, context: &mut C)
        -> rotor::Async<Self, Option<Self>>
    {
        match self {
            Wrapper::Incr(m) => m.wakeup(context).wrap(Wrapper::Incr),
            Wrapper::Get(m) => m.wakeup(context).wrap(Wrapper::Get),
        }
    }
}


fn main() {
    let mut event_loop = mio::EventLoop::new().unwrap();
    let mut handler = rotor::Handler::new(Context {
        counter: 0,
    }, &mut event_loop);
    handler.add_root(&mut event_loop,
        Wrapper::Incr(HttpServer::new(
            mio::tcp::TcpListener::bind(
                &"127.0.0.1:8888".parse().unwrap()).unwrap(),
            )));
    handler.add_root(&mut event_loop,
        Wrapper::Get(HttpServer::new(
            mio::tcp::TcpListener::bind(
                &"127.0.0.1:8889".parse().unwrap()).unwrap(),
            )));
    event_loop.run(&mut handler).unwrap();
}
