use hyper::version::HttpVersion::Http10;
use hyper::status::StatusCode::{self, PayloadTooLarge, BadRequest};
use hyper::status::StatusCode::{LengthRequired};
use hyper::header::{ContentLength, TransferEncoding, Connection};
use hyper::header::{Encoding, ConnectionOption};
use hyper::method::Method::{Get, Head};

use super::request::Head;


#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BodyKind {
    Fixed(u64),
    Upgrade,
    Chunked,
    Eof,
}

impl BodyKind {
    pub fn parse(head: &Head) -> Result<BodyKind, StatusCode> {
        use self::BodyKind::*;
        if let Some(&ContentLength(x)) = head.headers.get::<ContentLength>() {
            Ok((Fixed(x)))
        } else if let Some(items) = head.headers.get::<TransferEncoding>() {
            // TODO(tailhook) find out whether transfer encoding can be empty
            if &items[..] != [Encoding::Chunked] {
                Err(BadRequest)
            } else {
                Ok((Chunked))
            }
        } else if head.method == Get || head.method == Head {
            Ok(Fixed(0))
        } else if let Some(conn) = head.headers.get::<Connection>() {
            if conn.iter().find(|&x| *x == ConnectionOption::Close).is_some() {
                Ok(Eof)
            } else {
                Err(LengthRequired)
            }
        } else if head.version == Http10 {
            Ok(Eof)
        } else {
            Err(LengthRequired)
        }
    }
}
