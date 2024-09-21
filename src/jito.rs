use fastwebsockets::OpCode;
use futures_util::StreamExt;
use jito_searcher_client::get_searcher_client;
use log::debug;
use serde_json::json;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::EncodableKey;
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

use crate::util::env;
use crate::ws::connect_to_jito_tip_websocket;

pub type SearcherClient =
    SearcherServiceClient<InterceptedService<Channel, ClientInterceptor>>;

pub async fn make_searcher_client(
) -> Result<SearcherClient, Box<dyn std::error::Error>> {
    let auth_keypair =
        Arc::new(Keypair::read_from_file(env("AUTH_KEYPAIR_PATH"))?);
    let block_engine_url = env("BLOCK_ENGINE_URL");
    let searcher_client =
        get_searcher_client(&block_engine_url, &auth_keypair).await?;
    Ok(searcher_client)
}

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

#[timed::timed(duration(printer = "info!"))]
pub async fn send_out_bundle_to_all_regions(
    bundle: &[Transaction],
) -> Result<Vec<JoinHandle<()>>, Box<dyn std::error::Error>> {
    let client = Arc::new(RwLock::new(reqwest::Client::new()));
    let leader_regions = ["amsterdam", "ny", "frankfurt", "tokyo", "slc"];
    let leader_urls = leader_regions
        .iter()
        .map(|region| {
            format!(
                "https://{}.mainnet.block-engine.jito.wtf/api/v1/bundles",
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

    Ok(leader_urls
        .iter()
        .map(|leader_url| {
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
                let status = res.status();
                debug!("Sent bundle to {}: {:?}", leader_url, res);
                let body =
                    res.json::<serde_json::Value>().await.expect("json");
                info!("Bundle ID: {}, {}", body["result"], status);
                debug!("response: {:?}", body);
            })
        })
        .collect::<Vec<_>>())
}

///     curl https://mainnet.block-engine.jito.wtf/api/v1/bundles -X POST -H "Content-Type: application/json" -d '
/// {
///   "jsonrpc": "2.0",
///   "id": 1,
///   "method": "getInflightBundleStatuses",
///   "params": [
///     [
///     "b31e5fae4923f345218403ac1ab242b46a72d4f2a38d131f474255ae88f1ec9a",
///     "e3c4d7933cf3210489b17307a14afbab2e4ae3c67c9e7157156f191f047aa6e8",
///     "a7abecabd9a165bc73fd92c809da4dc25474e1227e61339f02b35ce91c9965e2",
///     "e3934d2f81edbc161c2b8bb352523cc5f74d49e8d4db81b222c553de60a66514",
///     "2cd515429ae99487dfac24b170248f6929e4fd849aa7957cccc1daf75f666b54"
///     ]
///   ]
/// }
/// '
pub async fn get_bundle_status(
    bundle_id: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let res = client
        .post("https://mainnet.block-engine.jito.wtf/api/v1/bundles")
        .header("content-type", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getInflightBundleStatuses",
            "params": [[bundle_id]]
        }))
        .send()
        .await
        .expect("send bundle");

    info!(
        "{}, {}: {:?}",
        bundle_id,
        res.status(),
        res.json::<serde_json::Value>().await?
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use futures::future::join_all;
    use solana_client::nonblocking::rpc_client::RpcClient;
    use solana_sdk::signer::Signer;
    use solana_sdk::system_instruction::transfer;

    use crate::util::{get_jito_tip_pubkey, init_logger, read_fund_keypair};

    use super::*;

    #[tokio::test]
    async fn all_bundles() {
        dotenv::dotenv().ok();
        init_logger().expect("init logger");
        info!("testing jito send out");
        let keypair = read_fund_keypair();
        let rpc_client =
            RpcClient::new("https://api.mainnet-beta.solana.com".to_string());
        let tip = 50000;
        let transaction = Transaction::new_signed_with_payer(
            &[
                transfer(&keypair.pubkey(), &keypair.pubkey(), tip),
                transfer(&keypair.pubkey(), &get_jito_tip_pubkey(), tip),
            ],
            Some(&keypair.pubkey()),
            &[&keypair],
            rpc_client
                .get_latest_blockhash()
                .await
                .expect("latest blockhash"),
        );
        let handles = send_out_bundle_to_all_regions(&[transaction])
            .await
            .expect("send out bundle");
        join_all(handles).await;
    }
}
