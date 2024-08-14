use fastwebsockets::OpCode;
use futures_util::StreamExt;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use jito_protos::bundle::{bundle_result::Result, BundleResult};
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
                    Result::Accepted(_) => {
                        info!("Bundle {} accepted", bundle_id);
                    }
                    Result::Rejected(rejection) => {
                        info!(
                            "Bundle {} rejected: {:?}",
                            bundle_id, rejection
                        );
                    }
                    Result::Dropped(_) => {
                        info!("Bundle {} dropped", bundle_id);
                    }
                    Result::Processed(_) => {
                        info!("Bundle {} processed", bundle_id);
                    }
                    Result::Finalized(_) => {
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
                            *dynamic_tip = ((top_75th * 0.7 + top_95th * 0.3)
                                * 10e9)
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
