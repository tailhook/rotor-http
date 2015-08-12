extern crate hyper;
extern crate rotor_http;
extern crate rotor;
extern crate mio;

type StateMachine = rotor::transports::accept::Serve<
                        mio::tcp::TcpListener,
                        rotor::transports::greedy_stream::Stream<
                            mio::tcp::TcpStream,
                            rotor_http::http1::Client<HelloWorld>>>;

struct Context {
    channel: mio::Sender<rotor::handler::Notify<StateMachine>>,
}

struct HelloWorld;

impl rotor_http::http1::Handler for HelloWorld {
    fn request(req: rotor_http::http1::Request,
               res: &mut rotor_http::http1::ResponseBuilder)
    {
        match req.uri {
            hyper::uri::RequestUri::AbsolutePath(ref p) if &p[..] == "/" => {
                res.set_status(hyper::status::StatusCode::Ok);
                res.put_body("Hello World!");
            }
            hyper::uri::RequestUri::AbsolutePath(p) => {
                res.set_status(hyper::status::StatusCode::Ok);
                res.put_body(format!("Hello {}!", &p[1..]));
            }
            _ => {}  // Do nothing: not found or bad request
        }
    }
}


impl rotor::context::AsyncContext<StateMachine> for Context {
    fn async_channel<'x>(&'x self)
        -> &'x mio::Sender<rotor::handler::Notify<StateMachine>>
    {
        &self.channel
    }
}


fn main() {
    let mut event_loop = mio::EventLoop::new().unwrap();
    let mut handler = rotor::Handler::new(Context {
        channel: event_loop.channel(),
    });
    event_loop.channel().send(rotor::handler::Notify::NewMachine(
        rotor::transports::accept::Serve::Accept(
            mio::tcp::TcpListener::bind(
                &"127.0.0.1:8888".parse().unwrap()).unwrap()
            ))).unwrap();
    event_loop.run(&mut handler).unwrap();
}
