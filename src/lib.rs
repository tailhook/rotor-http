extern crate rotor;
extern crate httparse;
extern crate time;
extern crate rotor_stream;
#[macro_use] extern crate quick_error;
#[macro_use] extern crate matches;

pub mod server;
pub mod client;
#[cfg(not(feature = "shareddocs"))]
mod shared;
#[cfg(feature = "shareddocs")]
pub mod shared;

pub use rotor_stream::{Deadline, Accept, Stream};

/// A shortcut type for server state machine
pub type ServerFsm<M, L> = Accept<Stream<
    server::Parser<M, <L as rotor::mio::TryAccept>::Output>>, L>;
