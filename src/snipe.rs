use fastwebsockets::{Frame, OpCode, Payload};
use futures::StreamExt;
use jito_protos::searcher::SubscribeBundleResultsRequest;
use jito_searcher_client::get_searcher_client;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::hash::Hash;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::EncodableKey;
use tokio::sync::{Mutex, RwLock};

use crate::pump::PumpBuyRequest;
use crate::pump_service::{_handle_pump_buy, update_latest_blockhash};
use crate::util::{env, pubkey_to_string, string_to_pubkey};
use crate::ws::connect_to_pump_websocket;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::Arc;

#[derive(Debug, Deserialize, Serialize)]
pub struct NewCoin {
    #[serde(
        serialize_with = "pubkey_to_string",
        deserialize_with = "string_to_pubkey"
    )]
    pub mint: Pubkey,
    pub twitter: Option<String>,
    pub website: Option<String>,
    pub telegram: Option<String>,
    pub created_timestamp: u64,
    #[serde(
        serialize_with = "pubkey_to_string",
        deserialize_with = "string_to_pubkey"
    )]
    pub bonding_curve: Pubkey,
    #[serde(
        serialize_with = "pubkey_to_string",
        deserialize_with = "string_to_pubkey"
    )]
    pub associated_bonding_curve: Pubkey,
    pub virtual_token_reserves: u64,
    pub virtual_sol_reserves: u64,
    pub real_token_reserves: u64,
    pub real_sol_reserves: u64,
}

pub enum MessageType {
    TradeCreated,
    NewCoinCreated,
    Unknown,
}

pub fn get_message_type(data: &str) -> MessageType {
    if data.starts_with(r#"42["tradeCreated""#) {
        MessageType::TradeCreated
    } else if data.starts_with(r#"42["newCoinCreated""#) {
        MessageType::NewCoinCreated
    } else {
        MessageType::Unknown
    }
}

pub async fn snipe_pump(lamports: u64) -> Result<(), Box<dyn Error>> {
    let latest_blockhash = Arc::new(RwLock::new(Hash::default()));
    let wallet = Arc::new(
        Keypair::read_from_file(env("FUND_KEYPAIR_PATH"))
            .expect("read fund keypair"),
    );
    let auth =
        Arc::new(Keypair::read_from_file(env("AUTH_KEYPAIR_PATH")).unwrap());
    let searcher_client = Arc::new(Mutex::new(
        get_searcher_client(env("BLOCK_ENGINE_URL").as_str(), &auth)
            .await
            .expect("makes searcher client"),
    ));

    // make parametrized as lamports probably, this will be changed to dynamic
    // tip calculation soon
    let tip = 1_000_000;

    // poll for latest blockhash to trim 200ms
    let rpc_client = Arc::new(RpcClient::new(env("RPC_URL")));
    tokio::spawn(update_latest_blockhash(
        rpc_client.clone(),
        latest_blockhash.clone(),
    ));

    // poll for bundle results
    let mut bundle_results_stream = searcher_client
        .lock()
        .await
        .subscribe_bundle_results(SubscribeBundleResultsRequest {})
        .await
        .expect("subscribe bundle results")
        .into_inner();
    tokio::spawn(async move {
        while let Some(res) = bundle_results_stream.next().await {
            info!("Received bundle result: {:?}", res);
        }
    });

    let mut ws = connect_to_pump_websocket().await?;
    ws.set_writev(true);

    ws.write_frame(Frame::text(Payload::Borrowed(b"40".as_ref())))
        .await?;

    loop {
        let frame = ws.read_frame().await?;
        match frame.opcode {
            OpCode::Close => {
                warn!("received close opcode");
                break;
            }
            OpCode::Text => {
                let data = String::from_utf8(frame.payload.to_vec())?;
                match data.as_str() {
                    "2" => {
                        ws.write_frame(Frame::text(Payload::Borrowed(
                            "3".as_ref(),
                        )))
                        .await?;
                        info!("Heartbeat sent");
                    }
                    _ => {
                        let message_type = get_message_type(&data);
                        match message_type {
                            MessageType::NewCoinCreated => {
                                let latest_blockhash =
                                    latest_blockhash.clone();
                                let wallet = wallet.clone();
                                let searcher_client = searcher_client.clone();
                                tokio::spawn(async move {
                                    let json_parsable = data
                                        .trim_start_matches(
                                            r#"42["newCoinCreated","#,
                                        )
                                        .trim_end_matches(']');
                                    let coin: NewCoin =
                                        serde_json::from_str(json_parsable)
                                            .expect("parse coin");
                                    if !coin_filter(&coin) {
                                        return;
                                    }
                                    let mut searcher_client =
                                        searcher_client.lock().await;
                                    let latest_blockhash =
                                        latest_blockhash.read().await;
                                    _handle_pump_buy(
                                        PumpBuyRequest {
                                            mint: coin.mint,
                                            bonding_curve: coin.bonding_curve,
                                            associated_bonding_curve: coin
                                                .associated_bonding_curve,
                                            virtual_token_reserves: coin
                                                .virtual_token_reserves,
                                            virtual_sol_reserves: coin
                                                .virtual_sol_reserves,
                                            slot: None,
                                        },
                                        lamports,
                                        tip,
                                        &wallet.clone(),
                                        &mut searcher_client,
                                        &latest_blockhash,
                                        None, // TODO add deadline here too
                                        0,
                                        1,
                                    )
                                    .await
                                    .expect("handle pump buy");
                                });
                            }
                            MessageType::TradeCreated => {
                                // maybe at some point
                            }
                            MessageType::Unknown => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn coin_filter(coin: &NewCoin) -> bool {
    let timestamp_now_ms = chrono::Utc::now().timestamp_millis();
    info!("checking {}", coin.mint);
    // check if got the info under 200ms
    let thresh_ms = 250;
    if timestamp_now_ms as u64 - coin.created_timestamp > thresh_ms {
        info!(
            "FAIL: got info {} ms after creation, need under {}",
            timestamp_now_ms as u64 - coin.created_timestamp,
            thresh_ms
        );
        return false;
    }
    // check if it cointains all socials
    if coin.twitter.is_none()
        || coin.website.is_none()
        || coin.telegram.is_none()
    {
        info!("FAIL: missing socials");
        return false;
    }
    // check if they are unique
    if coin.twitter == coin.website
        || coin.twitter == coin.telegram
        || coin.website == coin.telegram
    {
        info!("FAIL: socials are not unique");
        return false;
    }

    info!("PASS: coin is good");

    true
}
