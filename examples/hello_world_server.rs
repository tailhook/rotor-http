extern crate rotor_http;
extern crate rotor;
extern crate mio;

type StateMachine = rotor::transports::accept::Serve<
                        mio::tcp::TcpListener,
                        rotor::transports::greedy_stream::Stream<
                            mio::tcp::TcpStream,
                            rotor_http::http1::Client>>;

struct Context {
    channel: mio::Sender<rotor::handler::Notify<StateMachine>>,
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
