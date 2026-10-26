use clap::ValueEnum;

#[derive(Clone, Debug, ValueEnum, Default)]
pub enum Network {
    #[default]
    Preview,
    Mainnet,
}

impl Network {
    pub fn portal_graphql_url(&self) -> &str {
        match self {
            Network::Preview => "https://api.treasury.preview.sundae.fi/graphql",
            Network::Mainnet => "https://api.treasury.sundae.fi/graphql",
        }
    }

    pub fn koios_base_url(&self) -> &str {
        match self {
            Network::Preview => "https://preview.koios.rest/api/v1",
            Network::Mainnet => "https://api.koios.rest/api/v1",
        }
    }

    /// Cardano network magic (used for the Pallas node handshake).
    pub fn testnet_magic(&self) -> u32 {
        match self {
            Network::Preview => 2,
            Network::Mainnet => 764_824_073,
        }
    }

    // 0 = testnet, 1 = mainnet (for address derivation)
    pub fn network_id(&self) -> u8 {
        match self {
            Network::Preview => 0,
            Network::Mainnet => 1,
        }
    }

    /// Offset such that `unix_time = slot + genesis_offset_secs` for all Shelley+ slots.
    /// Mainnet: Shelley started at slot 4492800 = Unix 1596059091 → offset = 1596059091 - 4492800 = 1591566291.
    /// Preview: genesis at 2022-10-25T00:00:00Z = 1666656000 (slot 0 = that time).
    pub fn genesis_offset_secs(&self) -> u64 {
        match self {
            Network::Preview => 1_666_656_000,
            Network::Mainnet => 1_591_566_291,
        }
    }
}
