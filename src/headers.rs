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

#[cfg(test)]
mod test {
    use super::{is_content_length, is_transfer_encoding, is_connection};

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
}

