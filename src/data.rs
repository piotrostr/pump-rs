use std::str::FromStr;

use log::{info, warn};
use serde_json::json;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_request::RpcRequest;
use solana_client::rpc_response::RpcContactInfo;
use solana_sdk::pubkey::Pubkey;

use crate::constants::JITO_TIP_PUBKEY;

use tokio::time::{timeout, Duration};

#[timed::timed]
pub async fn look_for_rpc_nodes() {
    let rpc_client =
        RpcClient::new("https://api.mainnet-beta.solana.com".to_string());
    let nodes = rpc_client
        .send::<Vec<RpcContactInfo>>(RpcRequest::GetClusterNodes, json!([]))
        .await
        .unwrap();

    let futs = nodes.iter().map(|node| {
        let node = node.clone();
        async move {
            if let Some(rpc_url) = node.rpc.as_ref() {
                let rpc_client =
                    RpcClient::new(format!("http://{}", rpc_url));

                let handle = async {
                    if rpc_client
                        .get_account(
                            &Pubkey::from_str(JITO_TIP_PUBKEY).unwrap(),
                        )
                        .await
                        .is_ok()
                    {
                        info!("SUCCESS: {:?}", rpc_url);
                    }
                };

                let _ = timeout(Duration::from_secs(1), handle).await;
            }
        }
    });

    futures::future::join_all(futs).await;

    info!("Done, tried {}", nodes.len());
}
