use std::io;

use httparse;
use rotor_stream;


quick_error!{
    /// Error type which is passed to bad_response
    ///
    /// This is primarily for better debugging. Could also be used for putting
    /// into the logs.
    ///
    /// Note, you should not match the enum values and/or make an exhaustive
    /// match over the enum. More errors will be added at will.
    #[derive(Debug)]
    pub enum ResponseError {
        ChunkIsTooLarge(size: u64, limit: usize) {
            description("chunk size is larger than allowed")
            display("chunk size is {} but maximum is {}", size, limit)
        }
        InvalidChunkSize(e: httparse::InvalidChunkSize) {
            from()
            description("error parsing chunk size")
        }
        Connection(err: ProtocolError) {
            from()
            description("connection error")
            display("connection error: {}", err)
        }
    }
}

quick_error!{
    /// Error
    #[derive(Debug)]
    pub enum ProtocolError {
        /// Error when connecting
        ConnectError(err: io::Error) {
            description("connection error")
            display("connection error: {}", err)
        }
        /// Error on idle connection
        ConnectionClosed {
            description("connection closed")
            display("connection closed")
        }
        ReadError(err: io::Error) {
            description("error when reading from stream")
            display("read error: {}", err)
        }
        WriteError(err: io::Error) {
            description("error when writing to stream")
            display("write error: {}", err)
        }
    }
}

impl From<rotor_stream::Exception> for ProtocolError {
    fn from(e: rotor_stream::Exception) -> ProtocolError {
        use rotor_stream::Exception as S;
        use self::ProtocolError as D;
        match e {
            S::EndOfStream => D::ConnectionClosed,
            S::LimitReached => unreachable!(),
            S::ReadError(e) => D::ReadError(e),
            S::WriteError(e) => D::WriteError(e),
            S::ConnectError(e) => D::ConnectError(e),
        }
    }
}
