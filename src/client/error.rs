use httparse;


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
    }
}
