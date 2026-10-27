use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Listen on loopback unless an operator explicitly exposes the service.
    #[serde(default = "default_bind")]
    pub bind_address: std::net::IpAddr,

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

    /// Deadline for each node connection or query, in seconds.
    #[serde(default = "default_node_timeout")]
    pub node_timeout_secs: u64,

    /// Koios base URL for full history replay without a node and explicit
    /// transaction-registration lookup. Node history does not require Koios.
    pub koios_url: Option<String>,

    #[serde(default)]
    pub instances: Vec<InstanceConfig>,

    #[serde(default)]
    pub allowed_authors: Vec<String>,

    /// Permit unauthenticated registration only in explicit local development.
    #[serde(default)]
    pub development_auth_bypass: bool,

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
        let mut config: Self = toml::from_str(&text).context("parsing config toml")?;
        for author in &mut config.allowed_authors {
            *author = author.to_ascii_lowercase();
        }
        for instance in &mut config.instances {
            instance.id = instance.id.to_ascii_lowercase();
        }
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(self.port > 0, "port must be positive");
        anyhow::ensure!(!self.db_path.trim().is_empty(), "db_path must not be empty");
        anyhow::ensure!(
            (1..=60).contains(&self.node_timeout_secs),
            "node_timeout_secs must be between 1 and 60"
        );
        anyhow::ensure!(
            (1..=86_400).contains(&self.poll_interval_secs),
            "poll_interval_secs must be between 1 and 86400"
        );
        anyhow::ensure!(
            matches!(self.network_magic, 1 | 2 | 764_824_073),
            "unsupported network_magic; use 2 (preview), 1 (preprod), or 764824073 (mainnet)"
        );
        anyhow::ensure!(
            self.node_socket_path.is_some() || self.koios_url.is_some(),
            "configure node_socket_path, koios_url, or both"
        );
        if let Some(path) = &self.node_socket_path {
            anyhow::ensure!(
                !path.trim().is_empty(),
                "node_socket_path must not be empty"
            );
        }
        if let Some(url) = &self.koios_url {
            let parsed = reqwest::Url::parse(url).context("invalid koios_url")?;
            anyhow::ensure!(
                matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some(),
                "koios_url must be an absolute HTTP(S) URL"
            );
            anyhow::ensure!(
                parsed.username().is_empty() && parsed.password().is_none(),
                "koios_url must not contain credentials"
            );
            let host = parsed.host_str().unwrap();
            for (known, network) in [
                ("preview.koios.rest", "preview"),
                ("preprod.koios.rest", "preprod"),
                ("api.koios.rest", "mainnet"),
            ] {
                anyhow::ensure!(
                    host != known || network == self.network(),
                    "koios_url network disagrees with network_magic"
                );
            }
        }
        anyhow::ensure!(
            !self.development_auth_bypass || self.bind_address.is_loopback(),
            "development_auth_bypass requires a loopback bind_address"
        );
        self.registration_authors()?;
        for hash in &self.allowed_authors {
            anyhow::ensure!(
                hash.len() == 56 && hex::decode(hash).is_ok(),
                "allowed_authors entries must be 28-byte hexadecimal key hashes"
            );
        }
        let mut ids = std::collections::BTreeSet::new();
        for instance in &self.instances {
            anyhow::ensure!(
                instance.network == self.network(),
                "instance network disagrees with network_magic"
            );
            anyhow::ensure!(
                instance.id.len() == 56 && hex::decode(&instance.id).is_ok(),
                "instance id must be a 28-byte hexadecimal policy ID"
            );
            anyhow::ensure!(
                ids.insert(instance.id.to_ascii_lowercase()),
                "duplicate configured instance"
            );
        }
        Ok(())
    }

    pub fn registration_authors(&self) -> Result<&[String]> {
        anyhow::ensure!(self.development_auth_bypass || !self.allowed_authors.is_empty(),
            "allowed_authors is empty; configure authorized keys or explicitly enable development_auth_bypass on loopback");
        Ok(&self.allowed_authors)
    }

    /// The selected network is determined by validated chain magic, never URL text.
    pub fn network(&self) -> &'static str {
        match self.network_magic {
            1 => "preprod",
            764824073 => "mainnet",
            _ => "preview",
        }
    }
}

fn default_bind() -> std::net::IpAddr {
    std::net::Ipv4Addr::LOCALHOST.into()
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

fn default_node_timeout() -> u64 {
    30
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configuration(extra: &str) -> Config {
        toml::from_str(&format!(
            "koios_url = 'https://preview.koios.rest/api/v1'\n{extra}"
        ))
        .unwrap()
    }

    #[test]
    fn auth_requires_explicit_local_development_opt_in() {
        let config = configuration("");
        assert!(config.bind_address.is_loopback());
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("allowed_authors"));
        assert!(configuration("development_auth_bypass = true")
            .validate()
            .is_ok());
        assert!(
            configuration("development_auth_bypass = true\nbind_address = '0.0.0.0'")
                .validate()
                .is_err()
        );
        assert!(configuration(&format!(
            "allowed_authors = ['{}']\nbind_address = '0.0.0.0'",
            "01".repeat(28)
        ))
        .validate()
        .is_ok());
        assert!(configuration("allowed_authors = ['REPLACE_ME']")
            .validate()
            .is_err());
    }

    #[test]
    fn invalid_intervals_networks_backends_and_instance_settings_fail() {
        for extra in ["poll_interval_secs = 0", "node_timeout_secs = 0", "port = 0",
            "network_magic = 42", "network_magic = 1", "db_path = ''", "node_socket_path = ''",
            "[[instances]]\nid = 'bad'\nnetwork = 'preview'",
            "[[instances]]\nid = '00000000000000000000000000000000000000000000000000000000'\nnetwork = 'mainnet'"] {
            assert!(configuration(&format!("development_auth_bypass = true\n{extra}")).validate().is_err(), "{extra}");
        }
        let mut config = configuration("development_auth_bypass = true");
        config.koios_url = None;
        assert!(config.validate().is_err());
        config.node_socket_path = Some("/run/cardano/node.socket".into());
        assert!(config.validate().is_ok());
        for url in [
            "file:///etc/hosts",
            "https://user:password@example.invalid",
            "relative",
        ] {
            config.koios_url = Some(url.into());
            assert!(config.validate().is_err());
        }
    }
}
