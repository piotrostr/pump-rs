use fastwebsockets::{Frame, OpCode, Payload};
// use futures::StreamExt;
// use jito_protos::searcher::SubscribeBundleResultsRequest;
use jito_searcher_client::get_searcher_client;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::hash::Hash;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::EncodableKey;
use tokio::sync::{Mutex, RwLock};

use crate::pump::{mint_to_pump_accounts, PumpBuyRequest};
use crate::pump_service::{
    _handle_pump_buy, update_latest_blockhash, update_slot,
};
use crate::util::{env, pubkey_to_string, string_to_pubkey};
use crate::ws::connect_to_pump_portal_websocket;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::Arc;

#[derive(Serialize, Deserialize, Debug)]
pub struct NewPumpPortalToken {
    #[serde(
        serialize_with = "pubkey_to_string",
        deserialize_with = "string_to_pubkey"
    )]
    pub mint: Pubkey,
    #[serde(
        serialize_with = "pubkey_to_string",
        deserialize_with = "string_to_pubkey",
        rename = "traderPublicKey"
    )]
    pub dev: Pubkey,
    #[serde(
        serialize_with = "pubkey_to_string",
        deserialize_with = "string_to_pubkey",
        rename = "bondingCurveKey"
    )]
    pub bonding_curve: Pubkey,
    #[serde(rename = "vTokensInBondingCurve")]
    pub virtual_token_reserves: f64,
    #[serde(rename = "vSolInBondingCurve")]
    pub virtual_sol_reserves: f64,
    #[serde(rename = "initialBuy")]
    pub initial_buy: f64,
}

pub async fn snipe_portal(lamports: u64) -> Result<(), Box<dyn Error>> {
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
    let tip = 200000;

    // poll for latest blockhash to trim 200ms
    let rpc_client = Arc::new(RpcClient::new(env("RPC_URL")));
    tokio::spawn(update_latest_blockhash(
        rpc_client.clone(),
        latest_blockhash.clone(),
    ));
    let slot = Arc::new(RwLock::new(0));
    update_slot(slot.clone());

    // poll for bundle results
    // let mut bundle_results_stream = searcher_client
    //     .lock()
    //     .await
    //     .subscribe_bundle_results(SubscribeBundleResultsRequest {})
    //     .await
    //     .expect("subscribe bundle results")
    //     .into_inner();
    // tokio::spawn(async move {
    //     while let Some(res) = bundle_results_stream.next().await {
    //         info!("Received bundle result: {:?}", res);
    //     }
    // });

    let mut ws = connect_to_pump_portal_websocket().await?;
    ws.set_writev(true);

    let sub = r#"{"method":"subscribeNewToken"}"#;
    ws.write_frame(Frame::text(Payload::Bytes(sub.into())))
        .await
        .expect("write frame");

    loop {
        let frame = ws.read_frame().await?;
        match frame.opcode {
            OpCode::Close => {
                warn!("received close opcode");
                break;
            }
            OpCode::Text => {
                let data = String::from_utf8(frame.payload.to_vec())?;
                if data.contains("Successfully subscribed") {
                    continue;
                }
                let token: NewPumpPortalToken = serde_json::from_str(&data)?;
                let latest_blockhash = latest_blockhash.clone();
                let wallet = wallet.clone();
                let searcher_client = searcher_client.clone();
                let slot = slot.clone();
                tokio::spawn(async move {
                    let latest_blockhash = latest_blockhash.read().await;
                    let mut searcher_client = searcher_client.lock().await;
                    // below math is wrong, hardcoding for now
                    let virtual_token_reserves =
                        (token.virtual_token_reserves * 1e5).round() as u64;
                    let virtual_sol_reserves =
                        (token.virtual_sol_reserves * 1e9).round() as u64;
                    let pump_accounts = mint_to_pump_accounts(&token.mint);
                    let associated_bonding_curve =
                        pump_accounts.associated_bonding_curve;
                    // TODO buy based on this, say if
                    if virtual_sol_reserves > 31_000_000_000 {
                        warn!(
                            "dev bought >= 1 sol (vSOL: {})",
                            virtual_sol_reserves,
                        );
                        return;
                    }
                    let current_slot = slot.read().await;
                    info!("{} buying {}", current_slot, token.mint);
                    let buy_req = PumpBuyRequest {
                        mint: token.mint,
                        bonding_curve: token.bonding_curve,
                        associated_bonding_curve,
                        virtual_token_reserves,
                        virtual_sol_reserves,
                    };
                    _handle_pump_buy(
                        buy_req,
                        lamports,
                        tip,
                        &wallet.clone(),
                        &mut searcher_client,
                        &latest_blockhash,
                    )
                    .await
                    .expect("handle pump buy");
                });
            }
            _ => {}
        }
    }

    Ok(())
}
