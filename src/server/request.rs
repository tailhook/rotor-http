use std::net::SocketAddr;
use httparse;

use super::body::BodyKind;
use super::Version;


#[derive(Debug)]
/// Request headers
///
/// We don't have a request object because it is structured differently
/// based on whether it is buffered or streaming, chunked or http2, etc.
///
/// Note: we do our best to keep Head object same for HTTP 1-2 and HTTPS
pub struct Head<'a> {
    /// The client IP and port. If the connection is not using a standard
    /// TCP-IP connection this field will be `None`.
    pub client: Option<SocketAddr>,
    /// The HTTP protocol version.
    pub version: Version,
    /// The HTTP method. It is restricted to token chars.
    pub method: &'a str,
    /// The HTTP scheme is  a sequence of characters beginning with a
    /// letter and followed by any combination of letters, digits, plus,
    /// period or hyphen.
    pub scheme: &'a str,
    /// The path points to a specific resource.
    pub path: &'a str,
    /// A slice of HTTP headers.
    pub headers: &'a [httparse::Header<'a>],
    /// The body kind is either fixed, chunked or upgrade.
    pub body_kind: BodyKind,
}
