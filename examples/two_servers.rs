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
    channel: mio::Sender<rotor::handler::Notify<StateMachine>>,
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

impl rotor::context::AsyncAddMachine<StateMachine> for Context {
    fn async_add_machine(&mut self, m: StateMachine) -> Result<(), StateMachine> {
        rotor::send_machine(&mut self.channel, m)
    }
}

impl rotor::context::AsyncAddMachine<FSMIncr> for Context {
    fn async_add_machine(&mut self, m: FSMIncr) -> Result<(), FSMIncr> {
        if let Err(Wrapper::Incr(m)) = self.async_add_machine(Wrapper::Incr(m)) {
            Err(m)
        } else {
            Ok(())
        }
    }
}

impl rotor::context::AsyncAddMachine<FSMGet> for Context {
    fn async_add_machine(&mut self, m: FSMGet) -> Result<(), FSMGet> {
        if let Err(Wrapper::Get(m)) = self.async_add_machine(Wrapper::Get(m)) {
            Err(m)
        } else {
            Ok(())
        }
    }
}

impl<A:Send, B:Send, C> rotor::EventMachine<C> for Wrapper<A, B>
    where A: rotor::EventMachine<C>,
          B: rotor::EventMachine<C>,
{
    fn ready(self, events: mio::EventSet, context: &mut C) -> Option<Self> {
        match self {
            Wrapper::Incr(i) => i.ready(events, context).map(Wrapper::Incr),
            Wrapper::Get(i) => i.ready(events, context).map(Wrapper::Get),
        }
    }
    fn register<H: mio::Handler>(&mut self,
        tok: mio::Token, eloop: &mut mio::EventLoop<H>)
        -> Result<(), std::io::Error>
    {
        match self {
            &mut Wrapper::Incr(ref mut i) => i.register(tok, eloop),
            &mut Wrapper::Get(ref mut i) => i.register(tok, eloop),
        }
    }
}


fn main() {
    let mut event_loop = mio::EventLoop::new().unwrap();
    let mut handler = rotor::Handler::new(Context {
        channel: event_loop.channel(),
        counter: 0,
    });
    event_loop.channel().send(rotor::handler::Notify::NewMachine(
        Wrapper::Incr(rotor::transports::accept::Serve::new(
            mio::tcp::TcpListener::bind(
                &"127.0.0.1:8888".parse().unwrap()).unwrap(),
            )))).unwrap();
    event_loop.channel().send(rotor::handler::Notify::NewMachine(
        Wrapper::Get(rotor::transports::accept::Serve::new(
            mio::tcp::TcpListener::bind(
                &"127.0.0.1:8889".parse().unwrap()).unwrap(),
            )))).unwrap();
    event_loop.run(&mut handler).unwrap();
}
