extern crate hyper;
extern crate rotor_http;
extern crate rotor;
extern crate mio;


type StateMachine = rotor::transports::accept::Serve<
                        mio::tcp::TcpListener,
                        rotor::transports::greedy_stream::Stream<
                            mio::tcp::TcpStream,
                            rotor_http::http1::Client<Context, HelloWorld>,
                            Context>,
                        Context>;

struct Context {
    channel: mio::Sender<rotor::handler::Notify<StateMachine>>,
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

struct HelloWorld;

impl<C:Counter> rotor_http::http1::Handler<C> for HelloWorld {
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
            hyper::uri::RequestUri::AbsolutePath(ref p) if &p[..] == "/num"
            => {
                res.set_status(hyper::status::StatusCode::Ok);
                res.put_body(format!("This host visited {} times",
                                     ctx.get()));
            }
            hyper::uri::RequestUri::AbsolutePath(p) => {
                res.set_status(hyper::status::StatusCode::Ok);
                res.put_body(format!("Hello {}!", &p[1..]));
            }
            _ => {}  // Do nothing: not found or bad request
        }
    }
}


impl rotor::context::AsyncAddMachine<StateMachine> for Context {
    fn async_add_machine(&mut self, m: StateMachine)
        -> Result<(), StateMachine>
    {
        rotor::send_machine(&mut self.channel, m)
    }
}


fn main() {
    let mut event_loop = mio::EventLoop::new().unwrap();
    let mut handler = rotor::Handler::new(Context {
        channel: event_loop.channel(),
        counter: 0,
    });
    event_loop.channel().send(rotor::handler::Notify::NewMachine(
        rotor::transports::accept::Serve::new(
            mio::tcp::TcpListener::bind(
                &"127.0.0.1:8888".parse().unwrap()).unwrap(),
            ))).unwrap();
    event_loop.run(&mut handler).unwrap();
}
