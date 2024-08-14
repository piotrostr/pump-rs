use fastwebsockets::OpCode;
use futures_util::StreamExt;
use serde_json::json;
use solana_sdk::transaction::Transaction;
use solana_transaction_status::Encodable;
use solana_transaction_status::EncodedTransaction;
use solana_transaction_status::UiTransactionEncoding;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use jito_protos::bundle::{bundle_result, BundleResult};
use jito_protos::searcher::searcher_service_client::SearcherServiceClient;
use jito_protos::searcher::SubscribeBundleResultsRequest;
use jito_searcher_client::token_authenticator::ClientInterceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use tracing::info;

use crate::ws::connect_to_jito_tip_websocket;

pub type SearcherClient =
    SearcherServiceClient<InterceptedService<Channel, ClientInterceptor>>;

#[timed::timed(duration(printer = "info!"))]
pub async fn start_bundle_results_listener(
    searcher_client: Arc<Mutex<SearcherClient>>,
) {
    // keep a stream for bundle results
    // TODO hopefully this doesn't deadlock
    let mut bundle_results_stream = searcher_client
        .lock()
        .await
        .subscribe_bundle_results(SubscribeBundleResultsRequest {})
        .await
        .expect("subscribe bundle results")
        .into_inner();
    // poll for bundle results
    tokio::spawn(async move {
        while let Some(res) = bundle_results_stream.next().await {
            if let Ok(BundleResult {
                bundle_id,
                result: Some(result),
            }) = res
            {
                match result {
                    bundle_result::Result::Accepted(_) => {
                        info!("Bundle {} accepted", bundle_id);
                    }
                    bundle_result::Result::Rejected(rejection) => {
                        info!(
                            "Bundle {} rejected: {:?}",
                            bundle_id, rejection
                        );
                    }
                    bundle_result::Result::Dropped(_) => {
                        info!("Bundle {} dropped", bundle_id);
                    }
                    bundle_result::Result::Processed(_) => {
                        info!("Bundle {} processed", bundle_id);
                    }
                    bundle_result::Result::Finalized(_) => {
                        info!("Bundle {} finalized", bundle_id);
                    }
                }
            }
        }
    });
}

pub fn subscribe_tips(dynamic_tip: Arc<RwLock<u64>>) -> JoinHandle<()> {
    tokio::spawn({
        let dynamic_tip = dynamic_tip.clone();
        async move {
            let mut ws = connect_to_jito_tip_websocket()
                .await
                .expect("connect to jito ws");
            while let Ok(frame) = ws.read_frame().await {
                match frame.opcode {
                    OpCode::Text => {
                        if let Ok(payload) =
                            String::from_utf8(frame.payload.to_vec())
                        {
                            let payload_json =
                                &(serde_json::from_str::<serde_json::Value>(
                                    &payload,
                                )
                                .expect("parse payload"))[0];
                            let top_75th = payload_json
                                ["landed_tips_75th_percentile"]
                                .as_f64()
                                .unwrap();
                            let top_95th = payload_json
                                ["landed_tips_95th_percentile"]
                                .as_f64()
                                .unwrap();

                            let mut dynamic_tip = dynamic_tip.write().await;
                            *dynamic_tip =
                                ((top_75th * 0.95 + top_95th * 0.05) * 10e9)
                                    as u64;
                            info!("Updated tip to {}", dynamic_tip);
                        }
                    }
                    OpCode::Ping => {
                        info!("Received ping");
                    }
                    _ => {}
                }
            }
        }
    })
}

pub async fn send_out_bundle_to_all_regions(
    bundle: &[Transaction],
) -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(RwLock::new(reqwest::Client::new()));
    let leader_regions = [""]; // ["amsterdam", "ny", "frankfurt", "tokyo", "slc"];
    let leader_urls = leader_regions
        .iter()
        .map(|region| {
            format!(
                "https://{}mainnet.block-engine.jito.wtf/api/v1/bundles",
                region
            )
        })
        .collect::<Vec<String>>();

    let bundle = bundle
        .iter()
        .map(|tx| match tx.encode(UiTransactionEncoding::Binary) {
            EncodedTransaction::LegacyBinary(b) => b,
            _ => panic!("impossible"),
        })
        .collect::<Vec<_>>();

    for leader_url in leader_urls {
        let leader_url = leader_url.clone();
        let bundle = bundle.clone();
        let client = client.clone();
        tokio::spawn(async move {
            let client = client.read().await;
            let res = client
                .post(&leader_url)
                .header("content-type", "application/json")
                .json(&json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "sendBundle",
                    "params": [bundle]
                }))
                .send()
                .await
                .expect("send bundle");
            // .json::<serde_json::Value>()
            // .await
            // .expect("json");
            info!("{}", res.status());
            info!("Sent bundle to {}: {:?}", leader_url, res);
        });
    }
    Ok(())
}
