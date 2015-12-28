use hyper::status::StatusCode;
use rotor_stream::StreamSocket;
use time::Duration;

use super::Response;


pub trait Context {
    fn emit_error_page(&self,
        code: StatusCode, response: &mut Response)
    {
        unimplemented!();
    }
    fn byte_timeout(&self) -> Duration {
        Duration::seconds(10)
    }
}
