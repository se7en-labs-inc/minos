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

    // 0 = testnet, 1 = mainnet (for address derivation)
    pub fn network_id(&self) -> u8 {
        match self {
            Network::Preview => 0,
            Network::Mainnet => 1,
        }
    }
}
