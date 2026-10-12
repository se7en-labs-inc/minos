mod api;
mod config;
mod db;
mod indexer;
mod koios;
mod node;
mod parser;
mod schema;
mod sync;
mod types;

use anyhow::Result;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "minos_portal=info".into()))
        .init();

    let args: Vec<String> = std::env::args().collect();
    let config_path = args
        .iter()
        .find(|a| a.ends_with(".toml"))
        .cloned()
        .unwrap_or_else(|| "minos-portal.toml".to_string());

    // reset-sync subcommand: clear chain-sync checkpoint so history is replayed
    if args.iter().any(|a| a == "reset-sync") {
        let cfg = config::Config::load(&config_path)?;
        let store = db::Store::open(&cfg.db_path)?;
        for inst in &cfg.instances {
            store.reset_sync(&inst.id)?;
            info!("reset sync checkpoint for instance {}", inst.id);
        }
        info!("Done. Restart portal without reset-sync to replay from publish tx.");
        return Ok(());
    }

    let cfg = config::Config::load(&config_path)?;
    info!("Loaded config: {} instance(s)", cfg.instances.len());

    let store = Arc::new(db::Store::open(&cfg.db_path)?);

    let portal_node = cfg.node_socket_path.as_ref().map(|sock| {
        info!("Using node socket: {} (magic {})", sock, cfg.network_magic);
        Arc::new(
            node::PortalNode::new(sock.clone(), cfg.network_magic)
                .with_timeout(std::time::Duration::from_secs(cfg.node_timeout_secs)),
        )
    });

    let koios = cfg.koios_url.as_deref().map(|url| {
        info!("Koios enabled: {}", url);
        Arc::new(koios::Koios::new(url))
    });

    if portal_node.is_none() && koios.is_none() {
        anyhow::bail!("config must have at least one of node_socket_path or koios_url");
    }

    // Seed any instances listed in config (idempotent).
    let network = cfg.network();
    for inst in &cfg.instances {
        store.register_instance(&inst.id, network)?;
        info!("seeded instance {} ({})", inst.id, network);
    }

    // Spawn background poll indexer (live UTxO snapshots)
    {
        let store = store.clone();
        let cfg = cfg.clone();
        let pn = portal_node.clone();
        let k = koios.clone();
        tokio::spawn(async move {
            indexer::run(cfg, pn, k, store).await;
        });
    }

    // Spawn one chain-sync task per configured instance (historical + follow)
    if let Some(ref sock) = cfg.node_socket_path {
        for inst in &cfg.instances {
            let inst_id = inst.id.clone();
            let publish_tx = match store.get_instance(&inst_id)? {
                Some(i) if !i.publish_tx.is_empty() => i.publish_tx.clone(),
                _ => {
                    info!("instance {inst_id}: no publish_tx yet, skipping chain sync");
                    continue;
                }
            };
            let store2 = store.clone();
            let koios2 = koios.clone();
            let sock2 = sock.clone();
            let magic = cfg.network_magic;
            tokio::spawn(async move {
                sync::run(inst_id, publish_tx, sock2, magic, store2, koios2).await;
            });
        }
    }

    let schema = schema::build(store.clone(), koios, portal_node, cfg.clone());
    let app = api::router(schema);

    let addr = format!("0.0.0.0:{}", cfg.port);
    info!("Listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
