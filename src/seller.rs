use futures::StreamExt;
use jito_searcher_client::get_searcher_client;
use solana_client::nonblocking::pubsub_client::PubsubClient;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_config::{
    RpcTransactionConfig, RpcTransactionLogsConfig, RpcTransactionLogsFilter,
};
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::hash::Hash;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signature};
use solana_sdk::signer::{EncodableKey, Signer};
use solana_transaction_status::option_serializer::OptionSerializer;
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, UiTransactionEncoding,
};
use tokio::sync::{Mutex, RwLock};

use crate::pump::{mint_to_pump_accounts, sell_pump_token};
use crate::pump_service::{
    start_bundle_results_listener, update_latest_blockhash,
};
use crate::util::env;
use log::{info, warn};
use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;

pub async fn run_seller() -> Result<(), Box<dyn Error>> {
    let tip = 200000;
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

    // poll for latest blockhash to trim 200ms
    let rpc_client = Arc::new(RpcClient::new(env("RPC_URL")));
    tokio::spawn(update_latest_blockhash(
        rpc_client.clone(),
        latest_blockhash.clone(),
    ));

    start_bundle_results_listener(searcher_client.clone()).await;

    let pubsub_client = PubsubClient::new(&env("WS_URL")).await?;
    let (mut stream, unsub) = pubsub_client
        .logs_subscribe(
            RpcTransactionLogsFilter::Mentions(vec![wallet
                .pubkey()
                .to_string()]),
            RpcTransactionLogsConfig {
                commitment: Some(CommitmentConfig::processed()),
            },
        )
        .await?;
    while let Some(res) = stream.next().await {
        let sig = res.value.signature;
        let rpc_client = rpc_client.clone();
        let latest_blockhash = latest_blockhash.clone();
        let wallet = wallet.clone();
        let searcher_client = searcher_client.clone();
        tokio::spawn(async move {
            if let Ok(tx) = get_tx_with_retries(
                &rpc_client,
                &Signature::from_str(&sig).expect("sig"),
            )
            .await
            {
                // parse mint here and sell after waiting 10 slots
                let (mint, was_bid) =
                    tx_to_mint(&tx, &wallet.pubkey()).expect("tx to mint");
                if !was_bid {
                    return;
                }
                let ata = spl_associated_token_account::get_associated_token_address(
                    &wallet.pubkey(),
                    &mint,
                );
                println!("ata: {}, mint: {}", ata, mint);
                if let Ok(token_amount) =
                    get_token_balance_with_retries(&rpc_client, &ata).await
                {
                    if token_amount == 0 {
                        return;
                    }
                    let pump_accounts = mint_to_pump_accounts(&mint);
                    let mut searcher_client = searcher_client.lock().await;
                    let latest_blockhash = *latest_blockhash.read().await;
                    sell_pump_token(
                        &wallet,
                        latest_blockhash,
                        pump_accounts,
                        token_amount,
                        &mut searcher_client,
                        tip,
                    )
                    .await
                    .expect("sell pump token");
                    info!("Sold {} pump tokens", token_amount);
                } else {
                    warn!("Error getting token balance for {}", ata);
                }
            } else {
                warn!("Error getting transaction {}", sig);
            }
        });
    }
    unsub().await;
    Ok(())
}

#[timed::timed(duration(printer = "info!"))]
pub async fn get_tx_with_retries(
    rpc_client: &RpcClient,
    sig: &Signature,
) -> Result<
    EncodedConfirmedTransactionWithStatusMeta,
    Box<dyn Error + Send + Sync>,
> {
    let max_retries = 5;
    let mut cooldown = 200;
    for _ in 0..max_retries {
        match rpc_client
            .get_transaction_with_config(
                sig,
                RpcTransactionConfig {
                    encoding: Some(UiTransactionEncoding::Json),
                    commitment: Some(CommitmentConfig::confirmed()),
                    max_supported_transaction_version: None,
                },
            )
            .await
        {
            Ok(tx) => return Ok(tx),
            Err(e) => {
                warn!("Error getting transaction: {:?}", e);
                tokio::time::sleep(tokio::time::Duration::from_millis(
                    cooldown,
                ))
                .await;
                cooldown *= 2;
            }
        }
    }
    Err(format!("Error getting transaction {}", sig).into())
}

#[timed::timed(duration(printer = "info!"))]
pub async fn get_token_balance_with_retries(
    rpc_client: &RpcClient,
    ata: &Pubkey,
) -> Result<u64, Box<dyn Error + Send + Sync>> {
    let max_retries = 5;
    let mut cooldown = 200;
    for _ in 0..max_retries {
        match rpc_client
            .get_token_account_balance_with_commitment(
                ata,
                CommitmentConfig::processed(),
            )
            .await
        {
            Ok(balance) => {
                return Ok(balance
                    .value
                    .amount
                    .parse::<u64>()
                    .expect("parse u64"))
            }
            Err(e) => {
                warn!("Error getting token balance: {:?}", e);
                tokio::time::sleep(tokio::time::Duration::from_millis(
                    cooldown,
                ))
                .await;
                cooldown *= 2;
            }
        }
    }

    Err(format!("Error getting token balance for {}", ata).into())
}

#[timed::timed(duration(printer = "info!"))]
pub fn tx_to_mint(
    tx: &EncodedConfirmedTransactionWithStatusMeta,
    owner: &Pubkey,
) -> Result<(Pubkey, bool), Box<dyn Error>> {
    let mut is_bid = false;
    if let Some(meta) = &tx.transaction.meta {
        if let OptionSerializer::Some(logs) = &meta.log_messages {
            for log in logs {
                if log == "Program log: Instruction: Buy" {
                    is_bid = true;
                }
            }
        }
        if let OptionSerializer::Some(post_token_balances) =
            &meta.post_token_balances
        {
            for balance in post_token_balances {
                if let OptionSerializer::Some(account_owner) = &balance.owner
                {
                    if *account_owner != owner.to_string() {
                        continue;
                    }
                    return Ok((Pubkey::from_str(&balance.mint)?, is_bid));
                }
            }
        }
    }
    // this is incorrect, index fluctuates, but leaving the destructuring here
    // for future reference
    // if let EncodedTransaction::Json(tx) = &tx.transaction.transaction {
    //     if let UiMessage::Raw(msg) = &tx.message {
    //         if !msg.account_keys.contains(&owner.to_string()) {
    //             return Ok((Pubkey::default(), false));
    //         }
    //     }
    // }
    Ok((Pubkey::default(), is_bid))
}
