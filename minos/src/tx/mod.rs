pub mod adjudicate;
pub mod cbor_util;
pub mod complete;
pub mod fees;
pub mod fund;
pub mod metadata;
pub mod mint;
pub mod modify;
pub mod publish;
pub mod sign;
pub mod value;
pub mod withdraw;

use serde::{Deserialize, Serialize};

/// Unsigned transaction in cardano-cli compatible format.
/// Can be signed with `cardano-cli transaction sign` or any CIP-30 wallet.
#[derive(Debug, Serialize, Deserialize)]
pub struct UnsignedTx {
    #[serde(rename = "type")]
    pub ty: String,
    pub description: String,
    #[serde(rename = "cborHex")]
    pub cbor_hex: String,
}

impl UnsignedTx {
    pub fn new(description: impl Into<String>, cbor_hex: impl Into<String>) -> Self {
        Self {
            ty: "Unwitnessed Tx ConwayEra".to_string(),
            description: description.into(),
            cbor_hex: cbor_hex.into(),
        }
    }
}
