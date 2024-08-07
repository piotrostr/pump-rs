use chrono::Local;
use env_logger::Builder;
use futures::future::join_all;
use futures::StreamExt;
use jito_protos::searcher::SubscribeBundleResultsRequest;
use jito_searcher_client::get_searcher_client;
use log::LevelFilter;
use pump_rs::bench;
use pump_rs::constants::PUMP_FUN_MINT_AUTHORITY;
use pump_rs::snipe;
use solana_client::rpc_config::RpcTransactionConfig;
use solana_sdk::signature::Signature;
use solana_transaction_status::UiTransactionEncoding;
use std::collections::HashSet;
use std::io::Write;
use std::{error::Error, str::FromStr, sync::Arc, time::Duration};

use clap::Parser;
use pump_rs::{
    app::{App, Command},
    ata,
    pump::{self},
    pump_service,
    util::env,
};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey,
    signature::Keypair,
    signer::{EncodableKey, Signer},
};
use tokio::sync::{Mutex, Semaphore};

use log::{info, warn};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenv::from_filename(".env").unwrap();

    Builder::from_default_env()
        .format(|buf, record| {
            writeln!(
                buf,
                "{} [{}] [{}:{}] {}",
                Local::now().format("%Y-%m-%d %H:%M:%S"),
                record.level(),
                record.file().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                record.args()
            )
        })
        .filter(None, LevelFilter::Info)
        .try_init()?;

    let app = App::parse();

    match app.command {
        Command::Bench {} => {
            bench::bench_pump_connection().await?;
        }
        Command::Snipe { lamports } => {
            snipe::snipe(lamports).await?;
        }
        Command::Analyze { wallet_path } => {
            let keypair = Keypair::read_from_file(wallet_path)
                .expect("Failed to read wallet");
            let rpc_client =
                Arc::new(RpcClient::new(env("RPC_URL").to_string()));
            let pump_tokens =
                pump::get_tokens_held(&keypair.pubkey()).await?;
            let sniper_signatures = rpc_client
                .get_signatures_for_address(&keypair.pubkey())
                .await?;
            let sniper_signatures: Arc<HashSet<String>> =
                Arc::new(HashSet::from_iter(
                    sniper_signatures
                        .iter()
                        .map(|sig| sig.signature.to_string()),
                ));

            // Create a semaphore with 5 permits
            let semaphore = Arc::new(Semaphore::new(5));

            let results = pump_tokens.iter().map(|pump_token| {
                let rpc_client = rpc_client.clone();
                let sniper_signatures = sniper_signatures.clone();
                let pump_token = pump_token.clone();
                let sem = semaphore.clone();
                tokio::spawn(async move {
                    // Acquire a permit
                    let _permit = sem.acquire().await.unwrap();

                    let result = async {
                        let token_transactions = rpc_client
                            .get_signatures_for_address(
                                &Pubkey::from_str(&pump_token.mint)
                                    .expect("mint"),
                            )
                            .await
                            .expect("get signatures");

                        if token_transactions.is_empty() {
                            println!(
                                "No transactions found for {}",
                                pump_token.mint
                            );
                            return u64::MAX;
                        }

                        let first_tx_sig = token_transactions.last().unwrap();
                        let first_tx = rpc_client
                            .get_transaction_with_config(
                                &Signature::from_str(&first_tx_sig.signature)
                                    .expect("signature"),
                                RpcTransactionConfig {
                                    encoding: Some(
                                        UiTransactionEncoding::Json,
                                    ),
                                    commitment: None,
                                    max_supported_transaction_version: Some(
                                        0,
                                    ),
                                },
                            )
                            .await
                            .expect("get transaction");

                        let tx_sniped = token_transactions
                            .iter()
                            .filter(|sig| {
                                sniper_signatures
                                    .contains(&sig.signature.to_string())
                            })
                            .last();

                        if let Some(tx_sniped) = tx_sniped {
                            let json_tx = serde_json::to_value(&first_tx)
                                .expect("to json");
                            let is_mint_tx = json_tx["transaction"]
                                ["message"]["accountKeys"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .any(|key| {
                                    key.as_str().unwrap()
                                        == PUMP_FUN_MINT_AUTHORITY
                                });

                            if is_mint_tx {
                                let slots_difference =
                                    tx_sniped.slot - first_tx.slot;
                                println!(
                                    "{}: sniped in {} slots",
                                    pump_token.mint, slots_difference
                                );
                                slots_difference
                            } else {
                                println!(
                                    "No mint tx found for {}",
                                    pump_token.mint
                                );
                                u64::MAX
                            }
                        } else {
                            println!(
                                "No sniped tx found for {}",
                                pump_token.mint
                            );
                            u64::MAX
                        }
                    }
                    .await;

                    // Release the permit after 200ms (5 requests per second)
                    tokio::time::sleep(Duration::from_millis(200)).await;

                    result
                })
            });

            let results = join_all(results).await;
            let valid_results: Vec<u64> = results
                .into_iter()
                .flatten()
                .filter(|result| *result != u64::MAX)
                .collect();

            if !valid_results.is_empty() {
                let total_slots: u64 = valid_results.iter().sum();
                let average_slots =
                    total_slots as f64 / valid_results.len() as f64;
                println!("Total tokens analyzed: {}", pump_tokens.len());
                println!(
                    "Tokens successfully sniped: {}",
                    valid_results.len()
                );
                println!("Average snipe slots: {:.2}", average_slots);
            } else {
                println!("No valid results to calculate average");
            }
        }
        Command::Sanity {} => {
            let keypair = Keypair::read_from_file(env("FUND_KEYPAIR_PATH"))
                .expect("read wallet");
            let auth = Keypair::read_from_file(env("AUTH_KEYPAIR_PATH"))
                .expect("read auth");
            let rpc_client = RpcClient::new(env("RPC_URL").to_string());
            info!("Wallet: {}", keypair.pubkey());
            let balance = rpc_client.get_balance(&keypair.pubkey()).await?;
            info!("Balance: {}", balance);
            info!("Auth: {}", auth.pubkey());
            info!("RPC: {}", env("RPC_URL"));
            info!("Block Engine: {}", env("BLOCK_ENGINE_URL"));
        }
        Command::CloseTokenAccounts { wallet_path, burn } => {
            info!("Burn: {}", burn);
            let keypair =
                Keypair::read_from_file(wallet_path).expect("read wallet");
            info!("Wallet: {}", keypair.pubkey());
            let rpc_client =
                Arc::new(RpcClient::new(env("RPC_URL").to_string()));
            let auth = Arc::new(
                Keypair::read_from_file(env("AUTH_KEYPAIR_PATH")).unwrap(),
            );
            let mut searcher_client =
                get_searcher_client(env("BLOCK_ENGINE_URL").as_str(), &auth)
                    .await
                    .expect("makes searcher client");
            let mut bundle_results_stream = searcher_client
                .subscribe_bundle_results(SubscribeBundleResultsRequest {})
                .await
                .expect("subscribe bundle results")
                .into_inner();
            tokio::spawn(async move {
                while let Some(res) = bundle_results_stream.next().await {
                    info!("Received bundle result: {:?}", res);
                }
            });
            ata::close_all_atas(
                rpc_client,
                &keypair,
                burn,
                &mut searcher_client,
            )
            .await?;
        }
        Command::PumpService {} => {
            pump_service::run_pump_service().await?;
        }
        Command::BumpPump { mint } => {
            let keypair =
                Keypair::read_from_file("wtf.json").expect("read wallet");
            let rpc_client = RpcClient::new(env("RPC_URL").to_string());
            let auth = Arc::new(
                Keypair::read_from_file(env("AUTH_KEYPAIR_PATH")).unwrap(),
            );
            let mut searcher_client = Arc::new(Mutex::new(
                get_searcher_client(env("BLOCK_ENGINE_URL").as_str(), &auth)
                    .await
                    .expect("makes searcher client"),
            ));
            loop {
                match pump::send_pump_bump(
                    &keypair,
                    &rpc_client,
                    &Pubkey::from_str(&mint)?,
                    &mut searcher_client,
                    true,
                )
                .await
                {
                    Ok(_) => {
                        info!("Bump success");
                    }
                    Err(e) => {
                        warn!("Bump failed: {}", e);
                    }
                };

                tokio::time::sleep(Duration::from_secs(6)).await;
            }
        }
        Command::SweepPump { wallet_path } => {
            let keypair =
                Keypair::read_from_file(wallet_path).expect("read wallet");
            info!("Wallet: {}", keypair.pubkey());
            let rpc_client = RpcClient::new(env("RPC_URL").to_string());
            let pump_tokens =
                pump::get_tokens_held(&keypair.pubkey()).await?;
            info!("Tokens held: {}", pump_tokens.len());
            let auth = Arc::new(
                Keypair::read_from_file(env("AUTH_KEYPAIR_PATH")).unwrap(),
            );
            let mut searcher_client =
                get_searcher_client(env("BLOCK_ENGINE_URL").as_str(), &auth)
                    .await
                    .expect("makes searcher client");
            // poll for bundle results
            let mut bundle_results_stream = searcher_client
                .subscribe_bundle_results(SubscribeBundleResultsRequest {})
                .await
                .expect("subscribe bundle results")
                .into_inner();
            tokio::spawn(async move {
                while let Some(res) = bundle_results_stream.next().await {
                    info!("Received bundle result: {:?}", res);
                }
            });

            for pump_token in pump_tokens {
                let mint = Pubkey::from_str(&pump_token.mint)?;
                let pump_accounts =
                    pump::mint_to_pump_accounts(&mint).await?;
                if pump_token.balance > 0 {
                    // double-check balance of ata in order not to send a
                    // transaction bound to revert
                    let ata = spl_associated_token_account::get_associated_token_address(
                        &keypair.pubkey(),
                        &mint,
                    );
                    let actual_balance = match rpc_client
                        .get_token_account_balance(&ata)
                        .await
                    {
                        Ok(res) => res
                            .amount
                            .parse::<u64>()
                            .expect("balance: parse u64"),
                        Err(_) => {
                            warn!("No balance found for {}", mint);
                            0
                        }
                    };
                    if actual_balance > 0 {
                        info!(
                            "Selling {} of {}",
                            actual_balance, pump_token.mint
                        );
                        pump::sell_pump_token(
                            &keypair,
                            &rpc_client,
                            pump_accounts,
                            pump_token.balance,
                            &mut searcher_client,
                        )
                        .await?;
                        tokio::time::sleep(Duration::from_millis(300)).await;
                    }
                }
            }
        }
        Command::BuyPumpToken { mint: _ } => {
            return Err("Unimplemented".into());
        }
    }

    Ok(())
}
