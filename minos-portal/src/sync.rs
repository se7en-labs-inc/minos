//! Node-only historical replay and follow. Fresh instances start at origin;
//! neither a prior registration nor a Koios lookup is required.
use crate::{chain, db::Store};
use anyhow::{Context, Result};
use pallas_network::{
    facades::NodeClient,
    miniprotocols::{chainsync::NextResponse, Point},
};
use pallas_traverse::MultiEraBlock;
use std::sync::Arc;
use tracing::{info, warn};

pub async fn run(
    instance: String,
    socket: String,
    magic: u64,
    timeout_secs: u64,
    store: Arc<Store>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        if *shutdown.borrow() {
            break;
        }
        if let Err(error) = sync_once(
            &instance,
            &socket,
            magic,
            timeout_secs,
            &store,
            &mut shutdown,
        )
        .await
        {
            warn!("chain sync {instance}: {error:#}; retrying in 10s");
        }
        if *shutdown.borrow() {
            break;
        }
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {}
        }
    }
}
async fn sync_once(
    instance: &str,
    socket: &str,
    magic: u64,
    timeout_secs: u64,
    store: &Store,
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let timeout = std::time::Duration::from_secs(timeout_secs);
    let connection = tokio::time::timeout(timeout, NodeClient::connect(socket, magic));
    let mut client = tokio::select! {
        _ = shutdown.changed() => return Ok(()),
        connected = connection => connected.context("node connection timed out")??,
    };
    let result = tokio::select! {
        _ = shutdown.changed() => Ok(()),
        result = follow(instance, &mut client, timeout, store) => result,
    };
    client.abort().await;
    result
}
async fn follow(
    instance: &str,
    client: &mut NodeClient,
    timeout: std::time::Duration,
    store: &Store,
) -> Result<()> {
    let checkpoint = store.checkpoint(instance)?;
    let mut points = Vec::new();
    if let Some(point) = &checkpoint {
        points.push(Point::Specific(point.slot, hex::decode(&point.hash)?));
    }
    points.push(Point::Origin);
    let (intersection, _) =
        tokio::time::timeout(timeout, client.chainsync().find_intersect(points))
            .await
            .context("intersection timed out")??;
    match intersection.context("node rejected origin intersection")? {
        Point::Origin => store.rollback_chain(instance, None)?,
        Point::Specific(slot, hash) => {
            let point = chain::Point {
                slot,
                hash: hex::encode(hash),
                height: checkpoint.as_ref().map_or(0, |p| p.height),
            };
            store.rollback_chain(instance, Some(&point))?;
        }
    }
    info!("chain sync {instance}: replay/follow started");
    loop {
        // A healthy idle chain can wait indefinitely at tip. Timeout a pending
        // reply generously, then close the connection and resume durably.
        let next = tokio::time::timeout(timeout.max(std::time::Duration::from_secs(120)), async {
            if client.chainsync().has_agency() {
                client.chainsync().request_next().await
            } else {
                client.chainsync().recv_while_must_reply().await
            }
        })
        .await
        .context("chain reply timed out")??;
        match next {
            NextResponse::RollForward(raw, _) => {
                let block = MultiEraBlock::decode(raw.0.as_ref()).context("decode chain block")?;
                let transactions = block
                    .txs()
                    .into_iter()
                    .filter_map(|tx| tx.as_conway().map(|_| tx.encode()))
                    .collect();
                store.commit_block(
                    instance,
                    &chain::Block {
                        point: chain::Point {
                            slot: block.slot(),
                            hash: block.hash().to_string(),
                            height: block.number(),
                        },
                        transactions,
                    },
                )?;
            }
            NextResponse::RollBackward(point, _) => match point {
                Point::Origin => store.rollback_chain(instance, None)?,
                Point::Specific(slot, hash) => store.rollback_chain(
                    instance,
                    Some(&chain::Point {
                        slot,
                        hash: hex::encode(hash),
                        height: 0,
                    }),
                )?,
            },
            NextResponse::Await => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pallas_network::{
        miniprotocols::{
            chainsync::{self, ClientRequest, Tip},
            handshake::{self, n2c},
            PROTOCOL_N2C_HANDSHAKE,
        },
        multiplexer::{Bearer, Plexer},
    };

    #[tokio::test]
    async fn reconnect_recovers_stale_intersection_and_idle_shutdown_finishes() {
        let (instance, blocks) = crate::chain::tests::lifecycle_fixture();
        let store = Arc::new(Store::temporary());
        store.register_instance(&instance, "preview").unwrap();
        for block in &blocks {
            store.commit_block(&instance, block).unwrap();
        }
        let checkpoint = store.checkpoint(&instance).unwrap().unwrap();
        let expected = Point::Specific(checkpoint.slot, hex::decode(checkpoint.hash).unwrap());
        let path =
            std::env::temp_dir().join(format!("minos-portal-sync-{}.sock", std::process::id()));
        struct Socket(std::path::PathBuf);
        impl Drop for Socket {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _socket = Socket(path.clone());
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            // The first connection dies during handshake. The supervisor's
            // worker must reconnect without losing its durable checkpoint.
            let (failed, _) = Bearer::accept_unix(&listener).await.unwrap();
            drop(failed);
            let (bearer, _) = Bearer::accept_unix(&listener).await.unwrap();
            let mut plexer = Plexer::new(bearer);
            let mut handshake = handshake::Server::<n2c::VersionData>::new(
                plexer.subscribe_server(PROTOCOL_N2C_HANDSHAKE),
            );
            // Intersection/rollback/await have the same wire shape for N2C
            // and N2N; this mock never sends header or block content.
            let mut chain = chainsync::N2NServer::new(plexer.subscribe_server(5));
            let plexer = plexer.spawn();
            assert!(handshake
                .handshake(n2c::VersionTable::v10_and_above(2))
                .await
                .unwrap()
                .is_some());
            match chain.recv_while_idle().await.unwrap().unwrap() {
                ClientRequest::Intersect(points) => {
                    assert_eq!(points, vec![expected, Point::Origin])
                }
                other => panic!("expected intersection, got {other:?}"),
            }
            chain
                .send_intersect_found(Point::Origin, Tip(Point::Origin, 0))
                .await
                .unwrap();
            assert!(matches!(
                chain.recv_while_idle().await.unwrap(),
                Some(ClientRequest::RequestNext)
            ));
            chain
                .send_roll_backward(Point::Origin, Tip(Point::Origin, 0))
                .await
                .unwrap();
            assert!(matches!(
                chain.recv_while_idle().await.unwrap(),
                Some(ClientRequest::RequestNext)
            ));
            chain.send_await_reply().await.unwrap();
            ready_tx.send(()).unwrap();
            let _ = finish_rx.await;
            plexer.abort().await;
        });
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let worker = tokio::spawn(run(
            instance.clone(),
            path.to_str().unwrap().into(),
            2,
            1,
            store.clone(),
            shutdown_rx,
        ));
        tokio::time::timeout(std::time::Duration::from_secs(15), ready_rx)
            .await
            .unwrap()
            .unwrap();
        assert!(store.checkpoint(&instance).unwrap().is_none());
        assert!(store.projects_for_instance(&instance).unwrap().is_empty());
        assert!(store
            .get_tracked_instances()
            .unwrap()
            .iter()
            .any(|entry| entry.id == instance));
        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), worker)
            .await
            .unwrap()
            .unwrap();
        finish_tx.send(()).unwrap();
        server.await.unwrap();
    }
}
