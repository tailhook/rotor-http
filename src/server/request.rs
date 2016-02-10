use hyper::version::HttpVersion as Version;
use httparse;

use super::body::BodyKind;


#[derive(Debug)]
/// Request headers
///
/// We don't have a request object because it is structured differently
/// based on whether it is buffered or streaming, chunked or http2, etc.
///
/// Note: we do our base to keep Head object same for HTTP 1-2 and HTTPS
pub struct Head<'a> {
    // TODO(tailhook) add source IP address
    pub version: Version,
    pub https: bool,
    pub method: &'a str,
    pub path: &'a str,
    pub headers: &'a [httparse::Header<'a>],
    pub body_kind: BodyKind,
}
