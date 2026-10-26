//! Offline content addressing. This does not upload or pin the content.
use anyhow::{Context, Result};
use cryptoxide::{digest::Digest, sha2::Sha256};
use std::{io::Read, path::Path};

/// CIDv1, raw codec, sha2-256 multihash, lower-case unpadded base32.
/// A UnixFS/DAG-PB import is a different representation and may have a different CID.
pub fn raw_cid(mut input: impl Read) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = input.read(&mut buffer).context("read evidence bytes")?;
        if n == 0 {
            break;
        }
        hasher.input(&buffer[..n]);
    }
    let mut bytes = [0; 36];
    bytes[..4].copy_from_slice(&[1, 0x55, 0x12, 32]);
    hasher.result(&mut bytes[4..]);
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut cid = String::from("b");
    let mut accumulator = 0u32;
    let mut bits = 0;
    for byte in bytes {
        accumulator = (accumulator << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            cid.push(ALPHABET[((accumulator >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        cid.push(ALPHABET[((accumulator << (5 - bits)) & 31) as usize] as char);
    }
    Ok(cid)
}

pub fn file_url(path: &Path) -> Result<String> {
    let input = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    Ok(format!("ipfs://{}", raw_cid(input)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_raw_sha256_cid() {
        assert_eq!(
            raw_cid(&b""[..]).unwrap(),
            "bafkreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku"
        );
    }

    #[test]
    fn reader_chunking_does_not_change_cid() {
        let bytes = vec![42; 407_316];
        let normal = raw_cid(&bytes[..]).unwrap();
        let tiny = std::io::BufReader::with_capacity(7, &bytes[..]);
        assert_eq!(raw_cid(tiny).unwrap(), normal);
        let mut changed = bytes;
        changed[100] ^= 1;
        assert_ne!(raw_cid(&changed[..]).unwrap(), normal);
    }
}
