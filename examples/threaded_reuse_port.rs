extern crate hyper;
extern crate rotor_http;
extern crate rotor;
extern crate mio;
extern crate libc;

use std::env;
use std::thread;
use std::os::unix::io::AsRawFd;

type StateMachine<'a> = rotor::transports::accept::Serve<
                        mio::tcp::TcpListener,
                        rotor::transports::greedy_stream::Stream<
                            mio::tcp::TcpStream,
                            rotor_http::http1::Client<Context, HelloWorld>,
                            Context>,
                        Context>;
type Timeo = ();

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

fn main() {
    let threads = env::var("THREADS").unwrap_or("2".to_string())
        .parse().unwrap();
    let mut children = Vec::new();
    for _ in 0..threads {
        let sock = mio::tcp::TcpSocket::v4().unwrap();
        let one = 1i32;
        unsafe {
            assert!(libc::funcs::bsd43::setsockopt(
                sock.as_raw_fd(), libc::SOL_SOCKET,
                libc::SO_REUSEPORT,
                &one as *const libc::c_int as *const libc::c_void, 4) == 0);
        }
        sock.bind(&"127.0.0.1:6754".parse().unwrap()).unwrap();
        let lst = sock.listen(4096).unwrap();
        children.push(thread::spawn(move || {
            let mut event_loop = mio::EventLoop::new().unwrap();
            let mut handler = rotor::Handler::new(Context {
                counter: 0,
            }, &mut event_loop);
            event_loop.channel().send(rotor::handler::Notify::NewMachine(
                StateMachine::new(lst))).unwrap();
            event_loop.run(&mut handler).unwrap();
        }));
    }
    for child in children {
        child.join().unwrap();
    }
}
