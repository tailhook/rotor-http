/// The body kind of the HTTP message.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BodyKind {
    /// A fixed body length set by the `Content-Length` header.
    /// Messages without a body have the value `Fixed(0)`.
    Fixed(u64),
    /// A chunked body set by `Transfer-Encoding: chunked`.
    Chunked,
    /// The response is read until the connection is closed.
    ///
    /// Only used for legacy responses.
    Eof,
}
