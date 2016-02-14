//! **Private**: `shared` members are only for internal use.
//! Some types are reexposed in `server` and `client`.
//!
//! These items are private and only with `--features shareddocs`
//! documentation is created.

pub use self::recvmode::RecvMode;
pub use self::version::Version;

pub mod headers;
pub mod message;
mod recvmode;
mod version;
