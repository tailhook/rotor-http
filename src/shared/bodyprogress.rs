use super::{BodyKind, RecvMode};

pub enum BodyProgress {
    /// Buffered fixed-size request (bytes left)
    BufferFixed(usize),
    /// Buffered request till end of input (byte limit)
    BufferEof(usize),
    /// Buffered request with chunked encoding
    /// (limit, bytes buffered, bytes left for current chunk)
    BufferChunked(usize, usize, usize),
    /// Progressive fixed-size request (size hint, bytes left)
    ProgressiveFixed(usize, u64),
    /// Progressive till end of input (size hint)
    ProgressiveEof(usize),
    /// Progressive with chunked encoding
    /// (hint, offset, bytes left for current chunk)
    ProgressiveChunked(usize, usize, u64),
}

impl BodyProgress {
    pub fn start(mode: RecvMode, body: BodyKind) -> BodyProgress {
        use super::RecvMode::*;
        use super::BodyKind::*;
        use self::BodyProgress::*;

        match (mode, body) {
            (Buffered(x), Fixed(y)) => {
                assert!((x as u64) >= y);
                BufferFixed(y as usize)
            }
            (Buffered(x), Chunked) => BufferChunked(x, 0, 0),
            (Buffered(x), Eof) => BufferEof(x),
            (Progressive(x), Fixed(y)) => ProgressiveFixed(x, y),
            (Progressive(x), Chunked) => ProgressiveChunked(x, 0, 0),
            (Progressive(x), Eof) => ProgressiveEof(x),
        }
    }
}
