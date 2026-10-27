//! Supervise dynamically tracked instances. Snapshots report availability only;
//! confirmed state is produced exclusively by the shared chain reducer.
use crate::{config::Config, db::Store, koios::Koios, node::PortalNode};
use anyhow::Result;
use std::{collections::BTreeMap, sync::Arc};
use tokio::time::{sleep, Duration};
use tracing::warn;

pub async fn run(
    cfg: Config,
    portal_node: Option<Arc<PortalNode>>,
    koios: Option<Arc<Koios>>,
    store: Arc<Store>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let mut tasks = tokio::task::JoinSet::new();
    let mut running = BTreeMap::new();
    loop {
        if *shutdown.borrow() {
            break;
        }
        while let Some(result) = tasks.try_join_next_with_id() {
            let id = match result {
                Ok((id, ())) => id,
                Err(error) => {
                    warn!("instance task failed: {error}");
                    error.id()
                }
            };
            running.retain(|_, task| *task != id);
        }
        match store.get_tracked_instances() {
            Ok(instances) => {
                for instance in instances {
                    if running.contains_key(&instance.id) {
                        continue;
                    }
                    let id = instance.id.clone();
                    let store = store.clone();
                    let koios = koios.clone();
                    let cfg = cfg.clone();
                    let mut task_shutdown = shutdown.clone();
                    let task = tasks.spawn(async move {
                        if let Some(socket) = cfg.node_socket_path {
                            crate::sync::run(
                                instance.id,
                                socket,
                                cfg.network_magic,
                                cfg.node_timeout_secs,
                                store,
                                task_shutdown,
                            )
                            .await;
                        } else if let Some(koios) = koios {
                            loop {
                                if *task_shutdown.borrow() { break; }
                                let replay = tokio::select! {
                                    _ = task_shutdown.changed() => break,
                                    result = koios.replay(&instance.id, &instance.network, &store) => result,
                                };
                                if let Err(error) = replay
                                {
                                    warn!(
                                        "Koios replay {}: {error:#}; retaining committed state",
                                        instance.id
                                    );
                                    if let Err(error)=availability(&store,&instance.id,"QueryFailed") { warn!("persist unavailable state: {error:#}"); }
                                } else {
                                    if let Err(error)=availability(&store,&instance.id,"Available") { warn!("persist availability: {error:#}"); }
                                }
                                tokio::select! {
                                    _ = task_shutdown.changed() => break,
                                    _ = sleep(Duration::from_secs(cfg.poll_interval_secs)) => {}
                                }
                            }
                        }
                    });
                    running.insert(id, task.id());
                }
            }
            Err(error) => warn!("load tracked instances: {error:#}"),
        }
        if let Some(node) = &portal_node {
            let connection = tokio::select! {
                _ = shutdown.changed() => break,
                result = node.session() => result,
            };
            match connection {
                Ok(mut session) => {
                    let observation = tokio::select! {
                        _ = shutdown.changed() => None,
                        result = observe(&store, &mut session) => Some(result),
                    };
                    session.close().await;
                    let Some(observation) = observation else {
                        break;
                    };
                    if let Err(error) = observation {
                        warn!("node snapshot unavailable: {error:#}");
                        if let Err(error) = unavailable(&store) {
                            warn!("persist unavailable state: {error:#}");
                        }
                    }
                }
                Err(error) => {
                    warn!("node snapshot connection: {error:#}");
                    if let Err(error) = unavailable(&store) {
                        warn!("persist unavailable state: {error:#}");
                    }
                }
            }
        } else if let Some(koios) = &koios {
            let tip = tokio::select! {
                _ = shutdown.changed() => break,
                result = koios.tip() => result,
            };
            match tip {
                Ok(tip) => {
                    if let Err(error) = store
                        .set_sync("last_tip_slot", &tip.abs_slot.to_string())
                        .and_then(|_| {
                            store.set_sync("last_tip_height", &tip.block_height.to_string())
                        })
                    {
                        warn!("persist tip: {error:#}");
                    }
                }
                Err(error) => warn!("Koios tip: {error:#}"),
            }
        }
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = sleep(Duration::from_secs(cfg.poll_interval_secs)) => {}
        }
    }
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            warn!("instance task shutdown: {error}");
        }
    }
}
fn unavailable(store: &Store) -> Result<()> {
    for instance in store.get_tracked_instances()? {
        availability(store, &instance.id, "QueryFailed")?;
    }
    Ok(())
}
fn availability(store: &Store, instance: &str, status: &str) -> Result<()> {
    for project in store.projects_for_instance(instance)? {
        if let Some(output) = store.get_utxo(instance, &project.id)? {
            store.set_sync(&format!("availability_{}_{}",instance,project.id),&serde_json::json!({"status":status,"tx":output.tx_hash,"index":output.output_index}).to_string())?;
        }
    }
    Ok(())
}
async fn observe(store: &Store, session: &mut minos::node::NodeSession) -> Result<()> {
    let tip = session.tip().await?;
    let (slot, height) = (tip.abs_slot, tip.block_no);
    store.set_sync("last_tip_slot", &slot.to_string())?;
    store.set_sync("last_tip_height", &height.to_string())?;
    for instance in store.get_tracked_instances()? {
        for project in store.projects_for_instance(&instance.id)? {
            if let Some(output) = store.get_utxo(&instance.id, &project.id)? {
                let refs = vec![(output.tx_hash.clone(), output.output_index)];
                let result = session.utxo_info(&refs).await;
                let status = match &result {
                    Ok(outputs)
                        if outputs.iter().any(|u| {
                            u.tx_hash == output.tx_hash && u.tx_index == output.output_index
                        }) =>
                    {
                        "Available"
                    }
                    Ok(_) => "Unavailable",
                    Err(_) => "QueryFailed",
                };
                store.set_sync(&format!("availability_{}_{}", instance.id, project.id), &serde_json::json!({"status": status, "slot": slot, "tx": output.tx_hash, "index": output.output_index}).to_string())?;
                result?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn shutdown_cancels_a_stalled_supervisor_backend_request() {
        let started = Arc::new(tokio::sync::Notify::new());
        let observed = started.clone();
        let app = axum::Router::new().route(
            "/tip",
            axum::routing::post(move || {
                let observed = observed.clone();
                async move {
                    observed.notify_one();
                    std::future::pending::<String>().await
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let cfg: Config =
            toml::from_str(&format!("koios_url='{url}'\ndevelopment_auth_bypass=true")).unwrap();
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let worker = tokio::spawn(run(
            cfg,
            None,
            Some(Arc::new(Koios::new(&url))),
            Arc::new(Store::temporary()),
            shutdown_rx,
        ));
        tokio::time::timeout(Duration::from_secs(2), started.notified())
            .await
            .unwrap();
        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(2), worker)
            .await
            .unwrap()
            .unwrap();
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn supervisor_starts_new_instances_retries_errors_and_joins_on_shutdown() {
        use axum::{routing::post, Json, Router};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let requests = Arc::new(AtomicUsize::new(0));
        let seen = requests.clone();
        let app = Router::new()
            .route(
                "/tip",
                post(|| async {
                    Json(serde_json::json!([{"abs_slot":1,"block_no":1,"hash":"01".repeat(32)}]))
                }),
            )
            .route(
                "/asset_utxos",
                post(move |Json(body): Json<serde_json::Value>| {
                    let seen = seen.clone();
                    async move {
                        assert_eq!(body["_asset_list"][0][0], "ab".repeat(28));
                        seen.fetch_add(1, Ordering::SeqCst);
                        Json(serde_json::json!([]))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let cfg: Config = toml::from_str(&format!(
            "koios_url='{url}'\ndevelopment_auth_bypass=true\npoll_interval_secs=1"
        ))
        .unwrap();
        cfg.validate().unwrap();
        let store = Arc::new(Store::temporary());
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let worker = tokio::spawn(run(
            cfg,
            None,
            Some(Arc::new(Koios::new(&url))),
            store.clone(),
            shutdown_rx,
        ));
        tokio::task::yield_now().await;
        assert_eq!(requests.load(Ordering::SeqCst), 0);
        store
            .register_instance(&"ab".repeat(28), "preview")
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while requests.load(Ordering::SeqCst) < 2 {
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert!(store.checkpoint(&"ab".repeat(28)).unwrap().is_none());
        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(2), worker)
            .await
            .unwrap()
            .unwrap();
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn failed_queries_hide_stale_outputs_without_inventing_withdrawals() {
        let (instance, blocks) = crate::chain::tests::lifecycle_fixture();
        let store = Arc::new(Store::temporary());
        store.register_instance(&instance, "preview").unwrap();
        for block in blocks {
            store.commit_block(&instance, &block).unwrap();
        }
        let before = serde_json::to_value(store.projection(&instance).unwrap()).unwrap();
        availability(&store, &instance, "QueryFailed").unwrap();
        let cfg: Config =
            toml::from_str("koios_url='http://127.0.0.1:1'\ndevelopment_auth_bypass=true").unwrap();
        let schema = crate::schema::build(store.clone(), None, None, cfg);
        let query=format!("{{ instanceById(ID: \"{instance}\") {{ confirmed projects {{ identifier availability unspentOutput {{ transaction }} milestones {{ identifier status }} }} }} }}");
        let response = schema.execute(&query).await;
        assert!(response.errors.is_empty(), "{:?}", response.errors);
        let data = response.data.into_json().unwrap();
        let project = &data["instanceById"]["projects"][0];
        assert!(project["unspentOutput"].is_null());
        assert_eq!(project["availability"], "Unavailable");
        assert_eq!(project["milestones"][0]["status"], "Unavailable");
        assert_eq!(project["milestones"][1]["status"], "Withdrawn");
        assert_eq!(
            serde_json::to_value(store.projection(&instance).unwrap()).unwrap(),
            before
        );
        availability(&store, &instance, "Available").unwrap();
        let recovered = schema.execute(&query).await.data.into_json().unwrap();
        assert!(!recovered["instanceById"]["projects"][0]["unspentOutput"].is_null());
    }
}
