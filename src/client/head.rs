use httparse;
use super::Version;


#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BodyKind {
    Fixed(u64),
    Chunked,
    Eof,
}

pub struct Head<'a> {
    pub version: Version,
    pub code: u16,
    pub reason: &'a str,
    pub headers: &'a [httparse::Header<'a>],
    pub body_kind: BodyKind,
    pub close: bool,
}
