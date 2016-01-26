use hyper::version::HttpVersion as Version;
use hyper::status::StatusCode::{self, BadRequest};
use hyper::header::Headers;
use httparse;

use super::MAX_HEADERS_NUM;


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
    pub headers: Headers,
}

impl <'a>Head<'a> {
    pub fn parse(data: &[u8]) -> Result<Head, StatusCode> {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS_NUM];
        let mut raw = httparse::Request::new(&mut headers);
        match raw.parse(data) {
            Ok(httparse::Status::Complete(x)) => {
                assert!(x == data.len());
                Ok(Head {
                    https: false,
                    version: if raw.version.unwrap() == 1 { Version::Http11 }
                             else { Version::Http10 },
                    method: raw.method.unwrap(),
                    path: raw.path.unwrap(),
                    headers: try!(Headers::from_raw(raw.headers)
                        .map_err(|_| BadRequest)),
                })
            }
            Ok(_) => unreachable!(),
            Err(_) => {
                // Anything to do with error?
                // Should more precise errors be here?
                return Err(BadRequest);
            }
        }
    }
}
