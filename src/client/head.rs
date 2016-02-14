use httparse;
use super::{BodyKind, Version};

pub struct Head<'a> {
    pub version: Version,
    pub code: u16,
    pub reason: &'a str,
    pub headers: &'a [httparse::Header<'a>],
    pub body_kind: BodyKind,
    pub close: bool,
}
