extern crate hyper;
extern crate rotor_http;
#[macro_use] extern crate rotor;
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

struct Context {
    channel: mio::Sender<rotor::handler::Notify<Wrapper>>,
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

impl rotor::context::AsyncAddMachine<Wrapper> for Context {
    fn async_add_machine(&mut self, m: Wrapper) -> Result<(), Wrapper> {
        rotor::send_machine(&mut self.channel, m)
    }
}

rotor_compose_state_machines!(Wrapper<Context> {
    Incr(FSMIncr),
    Get(FSMGet),
});


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
