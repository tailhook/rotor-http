use std::ascii::AsciiExt;

#[inline(always)]
pub fn is_transfer_encoding(val: &str) -> bool {
    if val.len() != "transfer-encoding".len() {
        return false;
    }
    for (idx, ch) in val.bytes().enumerate() {
        if b"transfer-encoding"[idx] != ch.to_ascii_lowercase() {
            return false;
        }
    }
    return true;
}

#[inline(always)]
pub fn is_content_length(val: &str) -> bool {
    if val.len() != "content-length".len() {
        return false;
    }
    for (idx, ch) in val.bytes().enumerate() {
        if b"content-length"[idx] != ch.to_ascii_lowercase() {
            return false;
        }
    }
    return true;
}

#[inline(always)]
pub fn is_connection(val: &str) -> bool {
    if val.len() != "connection".len() {
        return false;
    }
    for (idx, ch) in val.bytes().enumerate() {
        if b"connection"[idx] != ch.to_ascii_lowercase() {
            return false;
        }
    }
    return true;
}

#[inline(always)]
// header value is byte sequence
// we need case insensitive comparison and strip out of the whitespace
pub fn is_close(val: &[u8]) -> bool {
    if val.len() < "close".len() {
        return false;
    }
    let mut iter = val.iter();
    for (idx, &ch) in iter.by_ref().enumerate() {
        match ch {
            b'\r' | b'\n' | b' ' | b'\t' => continue,
            b'c' | b'C' => {
                if idx + "close".len() > val.len() {
                    return false;
                }
                break;
            }
            _ => return false,
        }
    }
    for (idx, ch) in iter.by_ref().take(4).enumerate() {
        if b"lose"[idx] != ch.to_ascii_lowercase() {
            return false;
        }
    }
    return true;
}

#[inline(always)]
// header value is byte sequence
// we need case insensitive comparison and strip out of the whitespace
pub fn is_chunked(val: &[u8]) -> bool {
    if val.len() < "chunked".len() {
        return false;
    }
    let mut iter = val.iter();
    for (idx, &ch) in iter.by_ref().enumerate() {
        match ch {
            b'\r' | b'\n' | b' ' | b'\t' => continue,
            b'c' | b'C' => {
                if idx + "chunked".len() > val.len() {
                    return false;
                }
                break;
            }
            _ => return false,
        }
    }
    for (idx, ch) in iter.by_ref().take(6).enumerate() {
        if b"hunked"[idx] != ch.to_ascii_lowercase() {
            return false;
        }
    }
    return true;
}

#[cfg(test)]
mod test {
    use super::{is_content_length, is_transfer_encoding, is_connection};
    use super::{is_chunked, is_close};

    #[test]
    fn test_content_len() {
        assert!(is_content_length("Content-Length"));
        assert!(is_content_length("content-length"));
        assert!(is_content_length("CONTENT-length"));
        assert!(is_content_length("CONTENT-LENGTH"));
    }

    #[test]
    fn test_transfer_encoding() {
        assert!(is_transfer_encoding("Transfer-Encoding"));
        assert!(is_transfer_encoding("transfer-ENCODING"));
        assert!(is_transfer_encoding("TRANSFER-Encoding"));
        assert!(is_transfer_encoding("TRANSFER-ENCODING"));
    }

    #[test]
    fn test_connection() {
        assert!(is_connection("Connection"));
        assert!(is_connection("CONNECTION"));
        assert!(is_connection("ConneCTION"));
        assert!(is_connection("connection"));
    }

    #[test]
    fn test_chunked() {
        assert!(is_chunked(b"chunked"));
        assert!(is_chunked(b"Chunked"));
        assert!(is_chunked(b"chuNKED"));
        assert!(is_chunked(b"CHUNKED"));
        assert!(is_chunked(b"   CHUNKED"));
        assert!(is_chunked(b"   CHUNKED  "));
        assert!(is_chunked(b"chunked  "));
    }

    #[test]
    fn test_close() {
        assert!(is_close(b"close"));
        assert!(is_close(b"Close"));
        assert!(is_close(b"clOSE"));
        assert!(is_close(b"CLOSE"));
        assert!(is_close(b" CLOSE"));
        assert!(is_close(b"   close   "));
        assert!(is_close(b"Close   "));
    }
}

