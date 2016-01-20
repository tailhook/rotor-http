use hyper::version::HttpVersion as Version;
use hyper::status::StatusCode;
use hyper::header::Headers;


pub struct Head {
    pub version: Version,
    pub status: StatusCode,
    pub headers: Headers,
}
