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
    Ok(base32_cid(&bytes))
}

fn base32_cid(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut cid = String::from("b");
    let mut accumulator = 0u32;
    let mut bits = 0;
    for byte in bytes {
        accumulator = (accumulator << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            cid.push(ALPHABET[((accumulator >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        cid.push(ALPHABET[((accumulator << (5 - bits)) & 31) as usize] as char);
    }
    cid
}

fn varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 128 {
        out.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    out.push(value as u8);
}

fn protobuf_bytes(field: u8, value: &[u8], out: &mut Vec<u8>) {
    out.push(field * 8 + 2);
    varint(value.len() as u64, out);
    out.extend_from_slice(value);
}

fn sha256_cid(codec: u8, bytes: &[u8]) -> Vec<u8> {
    let mut cid = vec![1, codec, 0x12, 32];
    let mut digest = [0; 32];
    let mut hash = Sha256::new();
    hash.input(bytes);
    hash.result(&mut digest);
    cid.extend_from_slice(&digest);
    cid
}

/// CIDv1 balanced UnixFS, fixed 256-KiB chunks and raw leaves, no directory or
/// filesystem metadata. The evidence limit gives at most 40 links (below the
/// standard balanced fanout of 174), so a single parent suffices.
/// Encoding follows https://specs.ipfs.tech/unixfs/ and the DAG-PB specification.
pub fn unixfs_cid(bytes: &[u8]) -> Result<String> {
    const CHUNK: usize = 262_144;
    anyhow::ensure!(
        bytes.len() <= 10 * 1024 * 1024,
        "UnixFS evidence exceeds 10 MiB"
    );
    if bytes.len() <= CHUNK {
        return raw_cid(bytes);
    }
    let mut node = Vec::new();
    // Canonical UnixFS encodes links before the data field; unnamed links have
    // an explicitly empty Name, matching the Kubo balanced importer.
    for chunk in bytes.chunks(CHUNK) {
        let mut link = Vec::new();
        protobuf_bytes(1, &sha256_cid(0x55, chunk), &mut link);
        protobuf_bytes(2, b"", &mut link);
        link.push(24);
        varint(chunk.len() as u64, &mut link);
        protobuf_bytes(2, &link, &mut node);
    }
    let mut data = vec![8, 2, 24]; // UnixFS Type=File, filesize.
    varint(bytes.len() as u64, &mut data);
    for chunk in bytes.chunks(CHUNK) {
        data.push(32); // Unpacked blocksizes field.
        varint(chunk.len() as u64, &mut data);
    }
    protobuf_bytes(1, &data, &mut node);
    Ok(base32_cid(&sha256_cid(0x70, &node)))
}

/// Recompute a supported import representation from the exact embedded bytes.
pub fn verify_url(bytes: &[u8], url: &str) -> Result<&'static str> {
    if url == format!("ipfs://{}", raw_cid(bytes)?) {
        return Ok("raw-sha256");
    }
    anyhow::ensure!(
        url == format!("ipfs://{}", unixfs_cid(bytes)?),
        "PDF/CID mismatch"
    );
    Ok("unixfs-cidv1-256k-raw-leaves")
}

pub fn file_url(path: &Path) -> Result<String> {
    let input = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    Ok(format!("ipfs://{}", raw_cid(input)?))
}

pub fn unixfs_file_url(path: &Path) -> Result<String> {
    anyhow::ensure!(
        std::fs::metadata(path)?.len() <= 10 * 1024 * 1024,
        "UnixFS evidence exceeds 10 MiB"
    );
    let bytes = std::fs::read(path).with_context(|| format!("open {}", path.display()))?;
    Ok(format!("ipfs://{}", unixfs_cid(&bytes)?))
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

    #[test]
    fn unixfs_matches_independent_kubo_0401_vectors() {
        // ipfs add --only-hash --cid-version=1 --raw-leaves=true
        // --chunker=size-262144 --pin=false (files of zero bytes).
        for (size, cid) in [
            (
                0,
                "bafkreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku",
            ),
            (
                262_144,
                "bafkreiekhhjkxu4ztk3tyng3er3ijhg56mb44oe3gwbgquhzu4afrg2ksa",
            ),
            (
                262_145,
                "bafybeigllfqgfpqydppr6cmv56g7ax4wyhruzswvcefv6j5kj77nzttfki",
            ),
            (
                524_288,
                "bafybeig5axbv7uxiwv63kn6qvegw5vbz2tlgv4nhz4crv6g3f3rat7ttsi",
            ),
            (
                10_485_760,
                "bafybeicicmkwdi4ejuls6owvsrzcty5kht3ydya35eqf4i46yjgbs6xggy",
            ),
        ] {
            assert_eq!(unixfs_cid(&vec![0; size]).unwrap(), cid, "size {size}");
        }
        assert!(unixfs_cid(&vec![0; 10_485_761]).is_err());
    }
}
