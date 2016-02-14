use std::fmt::{self, Display};

/// Represents a version of the HTTP spec.
///
/// HTTP/0.9 is only of historic importance. It is not supported by
/// rotor-http and it will never be supported. Most requests that
/// appear to be HTTP/0.9 are malformed HTTP/1.0 requests.
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Version {
    /// HTTP/1.0 protocol version.
    Http10,
    /// HTTP/1.1 protocol version as described in RFC7230 and others.
    Http11,
    /// HTTP/2 protocol version as described in RFC7540.
    ///
    /// HTTP/2 switches HTTP to a binary transport and it is planned
    /// to support it in rotor-http.
    Http20,
}

impl Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use shared::Version::*;
        f.write_str(match *self {
            Http10 => "HTTP/1.0",
            Http11 => "HTTP/1.1",
            Http20 => "HTTP/2",
        })
    }
}
