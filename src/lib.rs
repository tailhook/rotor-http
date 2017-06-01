#![cfg_attr(feature="nightly", feature(test))]

extern crate rotor;
extern crate httparse;
extern crate rotor_stream;
#[cfg(feature="nightly")] extern crate test;
#[cfg(test)] extern crate rotor_test;
#[macro_use] extern crate quick_error;
#[macro_use] extern crate matches;
#[macro_use] extern crate log;

pub mod server;
pub mod client;
pub mod message;
mod recvmode;
mod headers;
mod version;
