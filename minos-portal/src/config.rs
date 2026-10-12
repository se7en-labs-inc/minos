use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default = "default_db_path")]
    pub db_path: String,

    /// Local cardano-node Unix socket path.
    /// When set, the indexer uses the node directly for tip + UTxO queries.
    pub node_socket_path: Option<String>,

    /// Cardano network magic (2 = preview, 1 = preprod, 764824073 = mainnet).
    #[serde(default = "default_network_magic")]
    pub network_magic: u64,

    /// Koios base URL. Optional — used for one-time publish-tx bootstrap
    /// (resolving tx hash → block slot for chain-sync start point).
    pub koios_url: Option<String>,

    #[serde(default)]
    pub instances: Vec<InstanceConfig>,

    #[serde(default)]
    pub allowed_authors: Vec<String>,

    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceConfig {
    pub id: String,
    pub network: String,
}

impl Config {
    pub fn load(path: &str) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading config {path}"))?;
        toml::from_str(&text).context("parsing config toml")
    }

    /// Derive Cardano network name from magic or Koios URL.
    pub fn network(&self) -> &'static str {
        match self.network_magic {
            1 => "preprod",
            764824073 => "mainnet",
            _ => {
                // Fall back to Koios URL hint if magic is ambiguous
                if let Some(url) = &self.koios_url {
                    if url.contains("preview") {
                        return "preview";
                    }
                    if url.contains("preprod") {
                        return "preprod";
                    }
                    if url.contains("mainnet") {
                        return "mainnet";
                    }
                }
                "preview"
            }
        }
    }
}

fn default_port() -> u16 {
    8080
}
fn default_db_path() -> String {
    "minos-portal.db".to_string()
}
fn default_poll_interval() -> u64 {
    20
}
fn default_network_magic() -> u64 {
    2
}
