use hyper::version::HttpVersion as Version;
use hyper::status::StatusCode;
use hyper::method::Method;
use hyper::uri::RequestUri;
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
pub struct Head {
    pub version: Version,
    pub https: bool,
    pub method: Method,
    pub uri: RequestUri,
    pub headers: Headers,
}

impl Head {
    pub fn parse(data: &[u8]) -> Result<Head, ()> {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS_NUM];
        let mut raw = httparse::Request::new(&mut headers);
        match raw.parse(data) {
            Ok(httparse::Status::Complete(x)) => {
                assert!(x == data.len());
                Ok(Head {
                    https: false,
                    version: if raw.version.unwrap() == 1 { Version::Http11 }
                             else { Version::Http10 },
                    method: try!(raw.method.unwrap().parse()
                        .map_err(|_| ())),
                    uri: try!(raw.path.unwrap().parse()
                        .map_err(|_| ())),
                    headers: try!(Headers::from_raw(raw.headers)
                        .map_err(|_| ())),
                })
            }
            Ok(_) => unreachable!(),
            Err(_) => {
                return Err(());
            }
        }
    }
}
