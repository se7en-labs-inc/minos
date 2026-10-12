use super::*;
use pallas_codec::utils::{AnyCbor, CborWrap, NonEmptyKeyValuePairs};
use pallas_network::{
    miniprotocols::{
        handshake, localstate::ClientQueryRequest, localtxsubmission, PROTOCOL_N2C_HANDSHAKE,
        PROTOCOL_N2C_STATE_QUERY, PROTOCOL_N2C_TX_SUBMISSION,
    },
    multiplexer::{Plexer, RunningPlexer},
};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tokio::{net::UnixListener, task::JoinHandle, time::timeout};

const ADDRESS: &str = "addr1v9w4tl2xpw0n85uxpdrd907pvm7agqqhq902uw320cmjgxc0e87ue";
static NEXT_SOCKET: AtomicU64 = AtomicU64::new(0);

// Pallas 1.4.0's TxValidationError encoder omits the outer ledger list which
// its decoder correctly expects. Use the actual node wire shape in this mock.
#[derive(Debug, minicbor::Encode, minicbor::Decode)]
#[cbor(transparent)]
struct WireRejection(#[n(0)] AnyCbor);
impl From<String> for WireRejection {
    fn from(s: String) -> Self {
        Self(AnyCbor::from_encode(s))
    }
}

struct NodeServer {
    plexer: RunningPlexer,
    query: localstate::Server,
    submission: localtxsubmission::GenericServer<EraTx, WireRejection>,
}
impl NodeServer {
    async fn accept(listener: &UnixListener, magic: u64) -> Result<Self> {
        let (bearer, _) = Bearer::accept_unix(listener).await?;
        let mut plexer = Plexer::new(bearer);
        let mut handshake = handshake::Server::<n2c::VersionData>::new(
            plexer.subscribe_server(PROTOCOL_N2C_HANDSHAKE),
        );
        let query = localstate::Server::new(plexer.subscribe_server(PROTOCOL_N2C_STATE_QUERY));
        let submission = localtxsubmission::GenericServer::new(
            plexer.subscribe_server(PROTOCOL_N2C_TX_SUBMISSION),
        );
        let plexer = plexer.spawn();
        assert!(handshake
            .handshake(n2c::VersionTable::v10_and_above(magic))
            .await?
            .is_some());
        Ok(Self {
            plexer,
            query,
            submission,
        })
    }
    fn statequery(&mut self) -> &mut localstate::Server {
        &mut self.query
    }
    fn localtxsubmission(&mut self) -> &mut localtxsubmission::GenericServer<EraTx, WireRejection> {
        &mut self.submission
    }
    async fn abort(self) {
        self.plexer.abort().await;
    }
}

struct Socket(std::path::PathBuf);
impl Drop for Socket {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn socket() -> (Socket, UnixListener, LocalNode) {
    let path = std::env::temp_dir().join(format!(
        "minos-node-{}-{}.sock",
        std::process::id(),
        NEXT_SOCKET.fetch_add(1, Ordering::Relaxed)
    ));
    let listener = UnixListener::bind(&path).unwrap();
    let node = LocalNode::new(path.to_str().unwrap().into(), 764824073);
    (Socket(path), listener, node)
}

async fn request(server: &mut NodeServer) -> q::Request {
    match server.statequery().recv_while_acquired().await.unwrap() {
        ClientQueryRequest::Query(cbor) => cbor.into_decode().unwrap(),
        other => panic!("expected query, got {other:?}"),
    }
}

async fn acquired(listener: UnixListener) -> NodeServer {
    let mut server = NodeServer::accept(&listener, 764824073).await.unwrap();
    assert!(server
        .statequery()
        .recv_while_idle()
        .await
        .unwrap()
        .is_some());
    server.statequery().send_acquired().await.unwrap();
    assert_eq!(
        request(&mut server).await,
        q::Request::LedgerQuery(q::LedgerQuery::HardForkQuery(
            q::HardForkQuery::GetCurrentEra
        ))
    );
    server
        .statequery()
        .send_result(AnyCbor::from_encode(6u16))
        .await
        .unwrap();
    server
}

async fn disconnected(mut server: NodeServer) {
    // Pallas abort may close the bearer before queued Release/Done are flushed.
    // Both graceful termination and EOF are valid; a leaked connection times out.
    timeout(Duration::from_secs(3), async {
        match server.statequery().recv_while_acquired().await {
            Ok(ClientQueryRequest::Release) => {
                assert!(!matches!(
                    server.statequery().recv_while_idle().await,
                    Ok(Some(_))
                ));
            }
            Err(_) => {}
            other => panic!("expected release or disconnect, got {other:?}"),
        }
    })
    .await
    .expect("node connection leaked");
    server.abort().await;
}

async fn joined(task: JoinHandle<()>) {
    timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
}

fn pparams() -> q::ProtocolParam {
    // ProtocolParam is a 31-field CBOR array; null starts each optional field empty.
    let mut cbor = vec![0x98, 31];
    cbor.extend([0xf6; 31]);
    let mut p: q::ProtocolParam = minicbor::decode(&cbor).unwrap();
    p.minfee_a = Some(44);
    p.minfee_b = Some(155381);
    p.max_transaction_size = Some(16384);
    p.collateral_percentage = Some(150);
    p.max_collateral_inputs = Some(3);
    p.protocol_version = Some((10, 0));
    p.max_tx_ex_units = Some(q::ExUnits {
        mem: 14_000_000,
        steps: 10_000_000_000,
    });
    p.cost_models_for_script_languages = Some(q::CostModels {
        plutus_v1: None,
        plutus_v2: None,
        plutus_v3: Some(vec![7, -2, 11]),
        unknown: vec![].into(),
    });
    p.execution_costs = Some(q::ExUnitPrices {
        mem_price: q::RationalNumber {
            numerator: 577,
            denominator: 10000,
        },
        step_price: q::RationalNumber {
            numerator: 721,
            denominator: 10000000,
        },
    });
    p.minfee_refscript_cost_per_byte = Some(q::RationalNumber {
        numerator: 31,
        denominator: 2,
    });
    p
}

fn outputs() -> q::UTxOByAddress {
    let addr = Address::from_bech32(ADDRESS).unwrap().to_vec();
    let datum = minicbor::decode::<q::PlutusData>(&hex::decode("d8799f182aff").unwrap()).unwrap();
    let asset_names =
        NonEmptyKeyValuePairs::try_from(vec![(vec![0, 255].into(), AnyUInt::U64(63188312000))])
            .unwrap();
    vec![
        (
            q::UTxO {
                transaction_id: [1; 32].into(),
                index: AnyUInt::U16(300),
            },
            q::TransactionOutput::Current(q::PostAlonsoTransactionOutput {
                address: addr.clone().into(),
                amount: q::Value::Multiasset(
                    AnyUInt::U32(7016680),
                    NonEmptyKeyValuePairs::try_from(vec![([2; 28].into(), asset_names)]).unwrap(),
                ),
                inline_datum: Some(q::DatumOption::Data(CborWrap(datum))),
                script_ref: Some(CborWrap(ScriptRef::PlutusV3Script(
                    localtxsubmission::primitives::PlutusScript(vec![0x43, 1, 2, 3].into()),
                ))),
            }),
        ),
        (
            q::UTxO {
                transaction_id: [3; 32].into(),
                index: AnyUInt::MajorByte(0),
            },
            q::TransactionOutput::Legacy(q::LegacyTransactionOutput {
                address: addr.into(),
                amount: q::Value::Coin(AnyUInt::U32(2000000)),
                datum_hash: None,
            }),
        ),
    ]
    .into()
}

#[test]
fn output_conversion_preserves_assets_datum_and_reference_bytes() {
    let out = decode_utxos(outputs()).unwrap();
    assert_eq!(out[0].address, ADDRESS);
    assert_eq!(out[0].tx_index, 300);
    assert_eq!(out[0].lovelace(), 7016680);
    assert_eq!(
        out[0].asset_list.as_ref().unwrap()[0].quantity,
        "63188312000"
    );
    assert_eq!(out[0].asset_list.as_ref().unwrap()[0].asset_name, "00ff");
    assert_eq!(out[0].datum_hex().as_deref(), Some("d8799f182aff"));
    assert_eq!(out[0].reference_script_size(), 4);
    assert_eq!(
        out[0].reference_script.as_ref().unwrap()["bytes"],
        "43010203"
    );
    assert_eq!(out[1].lovelace(), 2000000);
    assert!(out[1].datum_hex().is_none());
    assert_eq!(out[1].reference_script_size(), 0);
}

#[test]
fn protocol_conversion_requires_fields_and_preserves_cost_order() {
    let p = protocol_params(pparams()).unwrap();
    assert_eq!(p.v3_cost_model().unwrap(), vec![7, -2, 11]);
    assert_eq!(p.price_mem(), 0.0577);
    assert_eq!(p.price_step(), 0.0000721);
    assert_eq!(p.ref_script_cost_per_byte(), 16);
    let mut missing = pparams();
    missing.minfee_a = None;
    assert!(protocol_params(missing)
        .unwrap_err()
        .to_string()
        .contains("minfee_a"));
    let mut invalid = pparams();
    invalid
        .execution_costs
        .as_mut()
        .unwrap()
        .mem_price
        .denominator = 0;
    assert!(protocol_params(invalid).is_err());
}

#[tokio::test]
async fn invalid_references_fail_before_connecting() {
    let n = LocalNode::new("/nonexistent/minos-test.sock".into(), 764824073);
    assert!(n.utxo_info(&[]).await.unwrap().is_empty());
    assert!(n
        .utxo_info(&[("00".into(), 0)])
        .await
        .unwrap_err()
        .to_string()
        .contains("32 bytes"));
    assert!(n
        .address_utxos("bad address")
        .await
        .unwrap_err()
        .to_string()
        .contains("invalid query address"));
    assert!(n
        .submit_tx("zz")
        .await
        .unwrap_err()
        .to_string()
        .contains("invalid transaction hex"));
}

#[tokio::test]
async fn acquire_failure_disconnects() {
    let (_socket, listener, node) = socket();
    let server = tokio::spawn(async move {
        let mut s = NodeServer::accept(&listener, 764824073).await.unwrap();
        assert!(s.statequery().recv_while_idle().await.unwrap().is_some());
        s.statequery()
            .send_failure(localstate::AcquireFailure::PointTooOld)
            .await
            .unwrap();
        let done = timeout(Duration::from_secs(3), s.statequery().recv_while_idle())
            .await
            .unwrap();
        assert!(!matches!(done, Ok(Some(_))));
        s.abort().await;
    });
    assert!(node
        .tip()
        .await
        .unwrap_err()
        .to_string()
        .contains("acquire node ledger state"));
    joined(server).await;
}

#[tokio::test]
async fn cancelling_query_disconnects() {
    let (_socket, listener, node) = socket();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (cancelled_tx, cancelled_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut s = acquired(listener).await;
        assert_eq!(request(&mut s).await, q::Request::GetChainPoint);
        ready_tx.send(()).unwrap();
        cancelled_rx.await.unwrap();
        // Move back to Acquired so EOF can be observed with the server API.
        if s.statequery()
            .send_result(AnyCbor::from_encode(Point::Origin))
            .await
            .is_ok()
        {
            disconnected(s).await;
        } else {
            s.abort().await;
        }
    });
    let client = tokio::spawn(async move { node.tip().await });
    timeout(Duration::from_secs(3), ready_rx)
        .await
        .unwrap()
        .unwrap();
    client.abort();
    assert!(client.await.unwrap_err().is_cancelled());
    cancelled_tx.send(()).unwrap();
    joined(server).await;
}

#[tokio::test]
async fn origin_is_not_reported_as_a_current_tip() {
    let (_socket, listener, node) = socket();
    let server = tokio::spawn(async move {
        let mut s = acquired(listener).await;
        assert_eq!(request(&mut s).await, q::Request::GetChainPoint);
        s.statequery()
            .send_result(AnyCbor::from_encode(Point::Origin))
            .await
            .unwrap();
        disconnected(s).await;
    });
    assert!(node.tip().await.unwrap_err().to_string().contains("origin"));
    joined(server).await;
}

#[tokio::test]
async fn tip_uses_chain_slot_time_and_disconnects() {
    let (_socket, listener, node) = socket();
    let server = tokio::spawn(async move {
        let mut s = acquired(listener).await;
        assert_eq!(request(&mut s).await, q::Request::GetChainPoint);
        s.statequery()
            .send_result(AnyCbor::from_encode(Point::Specific(
                200129041,
                vec![4; 32],
            )))
            .await
            .unwrap();
        assert_eq!(request(&mut s).await, q::Request::GetChainBlockNo);
        s.statequery()
            .send_result(AnyCbor::from_encode(q::ChainBlockNumber {
                slot_timeline: 0,
                block_number: 14053123,
            }))
            .await
            .unwrap();
        disconnected(s).await;
    });
    let tip = node.tip().await.unwrap();
    assert_eq!(
        (tip.abs_slot, tip.block_no, tip.block_time),
        (200129041, 14053123, 1791695332)
    );
    joined(server).await;
}

#[tokio::test]
async fn protocol_query_uses_conway_and_disconnects_on_conversion_error() {
    let (_socket, listener, node) = socket();
    let server = tokio::spawn(async move {
        let mut s = acquired(listener).await;
        assert_eq!(
            request(&mut s).await,
            q::Request::LedgerQuery(q::LedgerQuery::BlockQuery(
                6,
                q::BlockQuery::GetCurrentPParams
            ))
        );
        let mut params = pparams();
        params.minfee_b = None;
        s.statequery()
            .send_result(AnyCbor::from_encode((params,)))
            .await
            .unwrap();
        disconnected(s).await;
    });
    assert!(node
        .protocol_params()
        .await
        .unwrap_err()
        .to_string()
        .contains("minfee_b"));
    joined(server).await;
}

#[tokio::test]
async fn address_and_reference_queries_are_filtered() {
    for by_address in [true, false] {
        let (_socket, listener, node) = socket();
        let refs = vec![(hex::encode([1; 32]), 300)];
        let expected = if by_address {
            q::BlockQuery::GetUTxOByAddress(vec![Address::from_bech32(ADDRESS)
                .unwrap()
                .to_vec()
                .into()])
        } else {
            q::BlockQuery::GetUTxOByTxIn(parse_refs(&refs).unwrap())
        };
        let server = tokio::spawn(async move {
            let mut s = acquired(listener).await;
            assert_eq!(
                request(&mut s).await,
                q::Request::LedgerQuery(q::LedgerQuery::BlockQuery(6, expected))
            );
            s.statequery()
                .send_result(AnyCbor::from_encode((outputs(),)))
                .await
                .unwrap();
            disconnected(s).await;
        });
        let out = if by_address {
            node.address_utxos(ADDRESS).await
        } else {
            node.utxo_info(&refs).await
        }
        .unwrap();
        assert_eq!(out[0].reference_script_size(), 4);
        joined(server).await;
    }
}

#[tokio::test]
async fn submission_preserves_body_hash_and_node_rejections() {
    // Synthetic empty body, only ever sent to the in-process mock server.
    let cbor = "84a3008001800200a0f5f6";
    for accepted in [true, false] {
        let (_socket, listener, node) = socket();
        let server = tokio::spawn(async move {
            let mut s = acquired(listener).await;
            match s.localtxsubmission().recv_next_request().await.unwrap() {
                localtxsubmission::Request::Submit(EraTx(6, bytes)) => {
                    assert_eq!(hex::encode(bytes), cbor)
                }
                other => panic!("unexpected submission: {other:?}"),
            }
            let response = if accepted {
                Response::Accepted
            } else {
                Response::Rejected(WireRejection(AnyCbor::from_encode(((
                    localtxsubmission::ShelleyBasedEra::Conway,
                    localtxsubmission::ApplyTxError(vec![
                        localtxsubmission::ConwayLedgerFailure::MempoolFailure(
                            "fixture rejection".into(),
                        ),
                    ]),
                ),))))
            };
            s.localtxsubmission()
                .send_submit_tx_response(response)
                .await
                .unwrap();
            disconnected(s).await;
        });
        let result = timeout(Duration::from_secs(5), node.submit_tx(cbor))
            .await
            .expect("submission protocol timed out");
        if accepted {
            assert_eq!(
                result.unwrap(),
                Hasher::<256>::hash(&hex::decode("a3008001800200").unwrap()).to_string()
            );
        } else {
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("fixture rejection"));
        }
        joined(server).await;
    }
}

#[tokio::test]
async fn stalled_handshake_times_out_and_closes_socket() {
    use tokio::io::AsyncReadExt;
    let (_socket, listener, node) = socket();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut handshake_bytes = Vec::new();
        timeout(
            Duration::from_secs(3),
            stream.read_to_end(&mut handshake_bytes),
        )
        .await
        .unwrap()
        .unwrap();
    });
    let err = node
        .with_timeout(Duration::from_millis(50))
        .tip()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("timed out"));
    joined(server).await;
}

#[tokio::test]
async fn stalled_queries_invalidate_shared_sessions_and_disconnect() {
    for _ in 0..3 {
        let (_socket, listener, node) = socket();
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let mut s = acquired(listener).await;
            assert_eq!(request(&mut s).await, q::Request::GetChainPoint);
            done_rx.await.unwrap();
            if s.statequery()
                .send_result(AnyCbor::from_encode(Point::Origin))
                .await
                .is_ok()
            {
                disconnected(s).await;
            } else {
                s.abort().await;
            }
        });
        let node = node.with_timeout(Duration::from_millis(200));
        let mut session = node.session().await.unwrap();
        assert!(session
            .tip()
            .await
            .unwrap_err()
            .to_string()
            .contains("timed out"));
        assert!(session
            .tip()
            .await
            .unwrap_err()
            .to_string()
            .contains("closed"));
        done_tx.send(()).unwrap();
        session.close().await;
        joined(server).await;
    }
}

#[tokio::test]
async fn submission_response_timeout_reports_unknown_hash_without_retry() {
    let cbor = "84a3008001800200a0f5f6";
    let (_socket, listener, node) = socket();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut s = acquired(listener).await;
        assert!(matches!(
            s.localtxsubmission().recv_next_request().await.unwrap(),
            localtxsubmission::Request::Submit(_)
        ));
        done_rx.await.unwrap();
        disconnected(s).await;
    });
    let err = node
        .with_timeout(Duration::from_millis(200))
        .submit_tx(cbor)
        .await
        .unwrap_err();
    match err.downcast_ref::<SubmissionError>().unwrap() {
        SubmissionError::Unknown { hash, .. } => assert_eq!(
            hash,
            &Hasher::<256>::hash(&hex::decode("a3008001800200").unwrap()).to_string()
        ),
        _ => panic!("a missing response is not a rejection"),
    }
    done_tx.send(()).unwrap();
    joined(server).await;
}

#[tokio::test]
async fn session_dropped_outside_runtime_still_disconnects() {
    let (_socket, listener, node) = socket();
    let server = tokio::spawn(async move {
        disconnected(acquired(listener).await).await;
    });
    let session = node.session().await.unwrap();
    std::thread::spawn(move || drop(session)).join().unwrap();
    joined(server).await;
}
