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
use tracing::info;

use crate::jito::subscribe_tips;
use crate::pump::{mint_to_pump_accounts, PumpBuyRequest};
use crate::pump_service::{
    _handle_pump_buy, update_latest_blockhash, BuyConfig,
};
use crate::slot::update_slot;
use crate::util::{env, pubkey_to_string, string_to_pubkey};
use crate::ws::connect_to_pump_portal_websocket;
use log::warn;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::Arc;

#[derive(Serialize, Deserialize, Debug)]
pub struct NewPumpPortalToken {
    signature: String,
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

    let dynamic_tip = Arc::new(RwLock::new(0));
    subscribe_tips(dynamic_tip.clone());

    // poll for latest blockhash to trim 200ms
    let rpc_client = Arc::new(RpcClient::new(env("RPC_URL")));
    tokio::spawn(update_latest_blockhash(
        rpc_client.clone(),
        latest_blockhash.clone(),
    ));
    let slot = Arc::new(RwLock::new(0));
    update_slot(slot.clone());

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
                info!("signature {:?}", token.signature);
                let latest_blockhash = latest_blockhash.clone();
                let wallet = wallet.clone();
                let searcher_client = searcher_client.clone();
                let slot = slot.clone();
                let dynamic_tip = dynamic_tip.clone();
                tokio::spawn(async move {
                    let latest_blockhash = latest_blockhash.read().await;
                    let mut searcher_client = searcher_client.lock().await;
                    // below math is wrong, hardcoding for now
                    let virtual_token_reserves =
                        (token.virtual_token_reserves * 1e6).round() as u64;
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
                    let current_slot = *slot.read().await;
                    info!("{} buying {}", current_slot, token.mint);
                    let buy_req = PumpBuyRequest {
                        mint: token.mint,
                        bonding_curve: token.bonding_curve,
                        associated_bonding_curve,
                        virtual_token_reserves,
                        virtual_sol_reserves,
                        slot: None,
                    };
                    // tl;dr
                    // if there is a deadline, might miss bids coz the pumpportal data comes before
                    // slot of creation
                    // --
                    // deadline is 7 slots but info comes before mint is finalized
                    // basically, if the token amount is based on the amount in the pool at
                    // launch, adding a low slippage param like 2% is going to ensure that even
                    // after people buy in and out only dev can dump, it might be useful to
                    // re-add the deadline onto the txs but there is a problem - sometimes the
                    // data comes before the slot of creation and sometimes after, it might be
                    // 10-20 slots before even and then up to 5 slots after, that means it
                    // might make sense to poll from the pump.fun api still, or get
                    // a jito validator, not sure how to go about this at this stage tbf
                    //
                    // if there is a problem with the deadline, either need to resolve it
                    _handle_pump_buy(
                        BuyConfig {
                            lamports,
                            tip: *dynamic_tip.read().await,
                            deadline: None,
                            jitter: 1,
                            num_tries: 1,
                        },
                        buy_req,
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
