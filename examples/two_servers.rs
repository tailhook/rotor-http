use std::io;
use std::marker::PhantomData;

extern crate hyper;
extern crate rotor_http;
extern crate rotor;
extern crate mio;


type FSMIncr = rotor::transports::accept::Serve<
                mio::tcp::TcpListener,
                rotor::transports::greedy_stream::Stream<
                    mio::tcp::TcpStream,
                    rotor_http::http1::Client<Context, Incr>,
                    Context>,
                Context>;
type FSMGet = rotor::transports::accept::Serve<
                mio::tcp::TcpListener,
                rotor::transports::greedy_stream::Stream<
                    mio::tcp::TcpStream,
                    rotor_http::http1::Client<Context, Get>,
                    Context>,
                Context>;
type StateMachine = Wrapper<FSMIncr, FSMGet>;

struct Context {
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

enum Wrapper<A, B> {
    Incr(A),
    Get(B),
}
struct ScopeIncr<'a, S: 'a, A, B, C>(&'a mut S, PhantomData<*const (A, B, C)>);
struct ScopeGet<'a, S: 'a, A, B, C>(&'a mut S, PhantomData<*const (A, B, C)>);

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


impl<'a, S, A, B, C> rotor::Scope<A> for ScopeIncr<'a, S, A, B, C>
    where S: rotor::Scope<Wrapper<A, B>> + 'a,
          A: rotor::EventMachine<C>,
          B: rotor::EventMachine<C>,
{
    fn async_add_machine(&mut self, m: A) -> Result<(), A> {
        self.0.async_add_machine(Wrapper::Incr(m))
        .map_err(|x| if let Wrapper::Incr(c) = x {
            c
        } else {
            unreachable!();
        })
    }
    fn add_timeout_ms(&mut self, delay: u64, t: A::Timeout)
        -> Result<mio::Timeout, mio::TimerError>
    {
        self.0.add_timeout_ms(delay, Wrapper::Incr(t))
    }
    fn clear_timeout(&mut self, timeout: mio::Timeout) -> bool {
        self.0.clear_timeout(timeout)
    }
    fn register<E: ?Sized>(&mut self, io: &E,
        interest: mio::EventSet, opt: mio::PollOpt)
        -> Result<(), io::Error>
        where E: mio::Evented
    {
        self.0.register(io, interest, opt)
    }
}

impl<'a, S, A, B, C> rotor::Scope<B> for ScopeGet<'a, S, A, B, C>
    where S: rotor::Scope<Wrapper<A, B>> + 'a,
          A: rotor::EventMachine<C>,
          B: rotor::EventMachine<C>,
{
    fn async_add_machine(&mut self, m: B) -> Result<(), B> {
        self.0.async_add_machine(Wrapper::Get(m))
        .map_err(|x| if let Wrapper::Get(c) = x {
            c
        } else {
            unreachable!();
        })
    }
    fn add_timeout_ms(&mut self, delay: u64, t: B::Timeout)
        -> Result<mio::Timeout, mio::TimerError>
    {
        self.0.add_timeout_ms(delay, Wrapper::Get(t))
    }
    fn clear_timeout(&mut self, timeout: mio::Timeout) -> bool {
        self.0.clear_timeout(timeout)
    }
    fn register<E: ?Sized>(&mut self, io: &E,
        interest: mio::EventSet, opt: mio::PollOpt)
        -> Result<(), io::Error>
        where E: mio::Evented
    {
        self.0.register(io, interest, opt)
    }
}
impl<A:Send, B:Send> rotor::BaseMachine for Wrapper<A, B>
    where A: rotor::BaseMachine,
          B: rotor::BaseMachine,
{
    type Timeout = Wrapper<A::Timeout, B::Timeout>;
}

impl<A:Send, B:Send, C> rotor::EventMachine<C> for Wrapper<A, B>
    where A: rotor::EventMachine<C>,
          B: rotor::EventMachine<C>,
{
    fn ready<S>(self, events: mio::EventSet, context: &mut C, scope: &mut S)
        -> Option<Self>
        where S: rotor::Scope<Self>
    {
        match self {
            Wrapper::Incr(i) => i.ready(events, context,
                &mut ScopeIncr(scope, PhantomData))
                .map(Wrapper::Incr),
            Wrapper::Get(i) => i.ready(events, context,
                &mut ScopeGet(scope, PhantomData))
                .map(Wrapper::Get),
        }
    }
    fn register<S>(&mut self, scope: &mut S)
        -> Result<(), io::Error>
        where S: rotor::Scope<Self>
    {
        match self {
            &mut Wrapper::Incr(ref mut i)
            => i.register(&mut ScopeIncr(scope, PhantomData)),
            &mut Wrapper::Get(ref mut i)
            => i.register(&mut ScopeGet(scope, PhantomData)),
        }
    }
    fn abort<S>(self, reason: rotor::handler::Abort,
        context: &mut C, scope: &mut S)
        where S: rotor::Scope<Self>
    {
        match self {
            Wrapper::Incr(i)
            => i.abort(reason, context, &mut ScopeIncr(scope, PhantomData)),
            Wrapper::Get(i)
            => i.abort(reason, context, &mut ScopeGet(scope, PhantomData)),
        }
    }
}


fn main() {
    let mut event_loop = mio::EventLoop::new().unwrap();
    let mut handler = rotor::Handler::new(Context {
        counter: 0,
    }, &mut event_loop);
    event_loop.channel().send(rotor::handler::Notify::NewMachine(
        Wrapper::Incr(FSMIncr::new(
            mio::tcp::TcpListener::bind(
                &"127.0.0.1:8888".parse().unwrap()).unwrap(),
            )))).unwrap();
    event_loop.channel().send(rotor::handler::Notify::NewMachine(
        Wrapper::Get(FSMGet::new(
            mio::tcp::TcpListener::bind(
                &"127.0.0.1:8889".parse().unwrap()).unwrap(),
            )))).unwrap();
    event_loop.run(&mut handler).unwrap();
}
