/// The expected type of request body, if any.
///
/// After the header fields are parsed the request body kind
/// is decided. This information can be useful for servers
/// to decide if a request should be accepted and if the request
/// should be received in buffered or progressive mode.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BodyKind {
    /// Fixed number of bytes body.
    ///
    /// A value of `Fixed(0)` is used for requests without body.
    Fixed(u64),
    /// The message body is transmitted as several chunks.
    ///
    /// The size of the message body is not yet known.
    Chunked,
    /// Reserved for future usage.
    Upgrade,
}
