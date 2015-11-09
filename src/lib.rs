extern crate mio;
extern crate rotor;
extern crate hyper;
extern crate httparse;
extern crate netbuf;
extern crate time;
#[macro_use] extern crate log;

pub mod http1;

use rotor::transports::{accept, stream};
use mio::tcp::{TcpListener, TcpStream};

pub type HttpServer<C, R> = accept::Serve<C,
                        TcpListener,
                        stream::Stream<C, TcpStream, http1::Client<C, R>>>;

