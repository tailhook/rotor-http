use hyper::status::StatusCode;
use hyper::header::{ContentLength, ContentType};
use time::Duration;
use hyper::mime::{Mime, TopLevel, SubLevel};

use super::Response;


pub trait Context {
    fn emit_error_page(&self,
        code: StatusCode, response: &mut Response)
    {
        response.status(code);
        let data = format!("<h1>{} {}</h1>\n\
            <p><small>Served for you by rotor-http</small></p>\n",
            code, code.canonical_reason().unwrap_or("Unknown"));
        let bytes = data.as_bytes();
        response.add_header(ContentLength(bytes.len() as u64)).unwrap();
        response.add_header(ContentType(
            Mime(TopLevel::Text, SubLevel::Html, vec![]))).unwrap();
        response.done_headers().unwrap();
        response.write_body(bytes);
        response.done();
    }
    fn byte_timeout(&self) -> Duration {
        Duration::seconds(10)
    }
}
