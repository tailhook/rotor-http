use hyper::status::StatusCode;
use time::Duration;

use super::Response;


pub trait Context {
    fn emit_error_page(&self, code: StatusCode, response: &mut Response) {
        response.status(code.to_u16(),
                        code.canonical_reason().unwrap_or("<unknown status code>"));
        let data = format!("<h1>{}</h1>\n\
            <p><small>Served for you by rotor-http</small></p>\n", code);
        let bytes = data.as_bytes();
        response.add_length(bytes.len() as u64).unwrap();
        response.add_header("Content-Type", b"text/html").unwrap();
        response.done_headers().unwrap();
        response.write_body(bytes);
        response.done();
    }
    fn byte_timeout(&self) -> Duration {
        Duration::seconds(10)
    }
}
