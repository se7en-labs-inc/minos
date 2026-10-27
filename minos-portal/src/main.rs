mod api;
mod chain;
mod config;
mod db;
mod indexer;
mod journal;
mod koios;
mod lineage;
mod maintenance;
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
    if args.get(1).is_some_and(|arg| arg == "backup") {
        anyhow::ensure!(
            args.len() == 4,
            "usage: minos-portal backup SOURCE_DB DESTINATION_JSON"
        );
        return maintenance::backup(&args[2], std::path::Path::new(&args[3]));
    }
    if args.get(1).is_some_and(|arg| arg == "restore") {
        anyhow::ensure!(
            args.len() == 4,
            "usage: minos-portal restore BACKUP_JSON NEW_DB"
        );
        return maintenance::restore(std::path::Path::new(&args[2]), &args[3]);
    }
    if args.get(1).is_some_and(|arg| arg == "migrate") {
        anyhow::ensure!(
            args.len() == 5,
            "usage: minos-portal migrate SOURCE_DB NEW_DB NETWORK"
        );
        anyhow::ensure!(
            ["preview", "preprod", "mainnet"].contains(&args[4].as_str()),
            "unsupported migration network"
        );
        return maintenance::migrate(&args[2], &args[3], &args[4]);
    }
    let config_path = args
        .iter()
        .find(|a| a.ends_with(".toml"))
        .cloned()
        .unwrap_or_else(|| "minos-portal.toml".to_string());

    // reset-sync subcommand: clear chain-sync checkpoint so history is replayed
    if args.iter().any(|a| a == "reset-sync") {
        let cfg = config::Config::load(&config_path)?;
        let store = db::Store::open(&cfg.db_path, cfg.network())?;
        let instances = store
            .get_tracked_instances()?
            .into_iter()
            .map(|instance| instance.id)
            .chain(cfg.instances.iter().map(|instance| instance.id.clone()))
            .collect::<std::collections::BTreeSet<_>>();
        let selection = reset_instance(&args)?;
        let selected = selection.as_deref();
        if let Some(selected) = selected {
            anyhow::ensure!(
                instances.contains(selected),
                "selected instance is not tracked"
            );
        }
        for instance in instances {
            if selected.is_none_or(|selected| selected == instance) {
                store.reset_sync(&instance)?;
                info!("reset sync checkpoint for instance {}", instance);
            }
        }
        info!("Done. Restart portal without reset-sync to replay history.");
        return Ok(());
    }

    let cfg = config::Config::load(&config_path)?;
    info!("Loaded config: {} instance(s)", cfg.instances.len());

    let store = Arc::new(db::Store::open(&cfg.db_path, cfg.network())?);

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
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let indexer = {
        let store = store.clone();
        let cfg = cfg.clone();
        let pn = portal_node.clone();
        let k = koios.clone();
        tokio::spawn(async move {
            indexer::run(cfg, pn, k, store, shutdown_rx).await;
        })
    };

    let schema = schema::build(store.clone(), koios, portal_node, cfg.clone());
    let app = api::router(schema);

    let addr = std::net::SocketAddr::new(cfg.bind_address, cfg.port);
    info!("Listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let signal_shutdown = shutdown_tx.clone();
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            if let Err(error) = tokio::signal::ctrl_c().await {
                tracing::warn!("shutdown signal: {error}");
            }
            let _ = signal_shutdown.send(true);
        })
        .await;
    let _ = shutdown_tx.send(true);
    let _ = indexer.await;
    result?;

    Ok(())
}

fn reset_instance(args: &[String]) -> Result<Option<String>> {
    let flags: Vec<_> = args
        .iter()
        .enumerate()
        .filter(|(_, arg)| *arg == "--instance")
        .collect();
    anyhow::ensure!(flags.len() <= 1, "--instance may only be supplied once");
    let Some((position, _)) = flags.first() else {
        return Ok(None);
    };
    let id = args
        .get(position + 1)
        .ok_or_else(|| anyhow::anyhow!("--instance requires a 28-byte instance ID"))?;
    anyhow::ensure!(
        id.len() == 56 && hex::decode(id).is_ok(),
        "--instance requires a 28-byte instance ID"
    );
    Ok(Some(id.to_ascii_lowercase()))
}

#[cfg(test)]
mod cli_tests {
    #[test]
    fn reset_rejects_missing_malformed_or_repeated_instance_before_clearing_state() {
        for args in [
            vec!["--instance".into()],
            vec!["--instance".into(), "reset-sync".into()],
            vec![
                "--instance".into(),
                "ab".repeat(28),
                "--instance".into(),
                "cd".repeat(28),
            ],
        ] {
            assert!(super::reset_instance(&args).is_err());
        }
        assert_eq!(
            super::reset_instance(&["--instance".into(), "AB".repeat(28)]).unwrap(),
            Some("ab".repeat(28))
        );
        assert_eq!(super::reset_instance(&[]).unwrap(), None);
    }
}
