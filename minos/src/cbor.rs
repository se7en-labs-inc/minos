//! Bound untrusted CBOR before allocating typed collections or recursing through
//! metadata. This accepts definite/indefinite containers; semantic compatibility
//! remains the responsibility of the transaction operation.
use anyhow::{ensure, Result};
use pallas_codec::minicbor::{data::Type, Decoder};

pub const MAX_TRANSACTION_BYTES: usize = 64 * 1024;

pub fn validate(bytes: &[u8]) -> Result<()> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_TRANSACTION_BYTES,
        "CBOR exceeds transaction size bound or is empty"
    );
    let mut decoder = Decoder::new(bytes);
    value(&mut decoder, 0, &mut 16_384)?;
    ensure!(decoder.position() == bytes.len(), "trailing CBOR bytes");
    Ok(())
}

fn value(decoder: &mut Decoder<'_>, depth: usize, remaining: &mut u64) -> Result<()> {
    ensure!(
        depth <= 64 && *remaining > 0,
        "CBOR exceeds depth/item bound"
    );
    *remaining -= 1;
    match decoder.datatype()? {
        Type::Array | Type::ArrayIndef => match decoder.array()? {
            Some(count) => {
                ensure!(count <= *remaining, "CBOR array exceeds item bound");
                for _ in 0..count {
                    value(decoder, depth + 1, remaining)?;
                }
            }
            None => {
                while decoder.datatype()? != Type::Break {
                    value(decoder, depth + 1, remaining)?;
                }
                decoder.skip()?;
            }
        },
        Type::Map | Type::MapIndef => match decoder.map()? {
            Some(count) => {
                ensure!(count <= *remaining / 2, "CBOR map exceeds item bound");
                for _ in 0..count {
                    value(decoder, depth + 1, remaining)?;
                    value(decoder, depth + 1, remaining)?;
                }
            }
            None => {
                while decoder.datatype()? != Type::Break {
                    value(decoder, depth + 1, remaining)?;
                    value(decoder, depth + 1, remaining)?;
                }
                decoder.skip()?;
            }
        },
        Type::Tag => {
            decoder.tag()?;
            value(decoder, depth + 1, remaining)?;
        }
        Type::BytesIndef => {
            for chunk in decoder.bytes_iter()? {
                chunk?;
                ensure!(*remaining > 0, "CBOR chunk limit");
                *remaining -= 1;
            }
        }
        Type::StringIndef => {
            for chunk in decoder.str_iter()? {
                chunk?;
                ensure!(*remaining > 0, "CBOR chunk limit");
                *remaining -= 1;
            }
        }
        Type::Break => anyhow::bail!("unexpected CBOR break"),
        _ => {
            decoder.skip()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_collection_size_chunks_and_trailing_data_are_bounded() {
        for depth in [1, 32, 65, 1024] {
            let mut nested = vec![0x9f; depth];
            nested.push(0);
            nested.extend(vec![0xff; depth]);
            assert_eq!(validate(&nested).is_ok(), depth <= 64);
        }
        assert!(validate(&[0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]).is_err());
        assert!(validate(&[0, 0]).is_err());
        assert!(validate(&[0xbf, 1, 0xff]).is_err());
        assert!(validate(&[0x5f, 0x41, 1, 0xff]).is_ok());
        assert!(validate(&[0x5f, 0x9f, 0xff, 0xff]).is_err());
        assert!(validate(&[0x7f, 0x61, b'x', 0xff]).is_ok());
        assert!(validate(&vec![0; MAX_TRANSACTION_BYTES + 1]).is_err());
    }
}
