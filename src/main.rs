use futures::future::join_all;
use futures::StreamExt;
use jito_protos::searcher::SubscribeBundleResultsRequest;
use jito_searcher_client::get_searcher_client;
use pump_rs::bench;
use pump_rs::constants::PUMP_FUN_MINT_AUTHORITY;
use pump_rs::constants::TOKEN_PROGRAM;
use pump_rs::data::look_for_rpc_nodes;
use pump_rs::jito::get_bundle_status;
use pump_rs::jito::subscribe_tips;
use pump_rs::seller;
use pump_rs::seller::get_tx_with_retries;
use pump_rs::slot::make_deadline_tx;
use pump_rs::slot::update_slot;
use pump_rs::snipe;
use pump_rs::snipe_portal;
use pump_rs::util::init_logger;
use pump_rs::util::parse_holding;
use pump_rs::wallet::WalletManager;
use solana_client::rpc_config::RpcSendTransactionConfig;
use solana_client::rpc_config::RpcTransactionConfig;
use solana_client::rpc_request::TokenAccountsFilter;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::signature::Signature;
use solana_transaction_status::UiTransactionEncoding;
use std::collections::HashSet;
use std::{error::Error, str::FromStr, sync::Arc, time::Duration};
use tokio::sync::RwLock;

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
    dotenv::from_filename(".env")?;
    init_logger()?;

    let app = App::parse();

    match app.command {
        Command::Wallets {} => {
            let owner = Keypair::read_from_file(env("FUND_KEYPAIR_PATH"))
                .expect("read wallet");
            let rpc_client =
                Arc::new(RpcClient::new(env("RPC_URL").to_string()));
            let manager = WalletManager::new(
                rpc_client.clone(),
                Some("../keys".to_string()),
                owner,
            );

            manager.balances().await?;
        }
        Command::LookForGeyser {} => {
            look_for_rpc_nodes().await;
        }
        Command::BundleStatus { bundle_id } => {
            get_bundle_status(bundle_id).await?;
        }
        Command::SubscribeTip {} => {
            let tip = Arc::new(RwLock::new(0u64));
            subscribe_tips(tip).await?;
        }
        Command::GetTx { sig } => {
            let signature = Signature::from_str(&sig).expect("parse sig");
            let rpc_client = RpcClient::new(env("RPC_URL").to_string());
            let tx = get_tx_with_retries(&rpc_client, &signature)
                .await
                .expect("tx");
            info!("{:#?}", tx);
        }
        Command::SlotCreated { mint } => {
            let rpc_client = RpcClient::new(env("RPC_URL").to_string());
            let slot_created = pump::get_slot_created(
                &rpc_client,
                &Pubkey::from_str(&mint)?,
            )
            .await?;
            info!("{}: created {}", mint, slot_created);
        }
        Command::SubscribePump {} => {
            let slot = Arc::new(RwLock::new(0));
            update_slot(slot.clone());
            let _ = pump::subscribe_to_pump(slot.clone()).await;
        }
        Command::TestSlotProgram {} => {
            let rpc_client = RpcClient::new(env("RPC_URL").to_string());
            let keypair = Keypair::read_from_file(env("FUND_KEYPAIR_PATH"))
                .expect("read wallet");
            let current_slot = rpc_client
                .get_slot_with_commitment(CommitmentConfig::confirmed())
                .await?;
            info!("Current slot: {}", current_slot);
            let tx = make_deadline_tx(
                current_slot + 20,
                rpc_client.get_latest_blockhash().await?,
                &keypair,
            );
            let sig = rpc_client
                .send_and_confirm_transaction_with_spinner_and_config(
                    &tx,
                    CommitmentConfig::confirmed(),
                    RpcSendTransactionConfig {
                        skip_preflight: true,
                        ..Default::default()
                    },
                )
                .await?;
            info!("{sig:#?}");
        }
        Command::SlotSubscribe {} => {
            let current_slot = Arc::new(RwLock::new(0));
            println!("Current slot: {}", *current_slot.read().await);
            tracing::info!(msg = "Subscribing to slot updates");
            let _ = update_slot(current_slot).await;
        }
        Command::IsOnCurve { pubkey } => {
            let pubkey = Pubkey::from_str(&pubkey).expect("parse pubkey");
            println!("Pubkey: {}", pubkey);
            println!("Is on curve: {}", pubkey.is_on_curve());
        }
        Command::Subscribe {} => {
            let auth = Arc::new(
                Keypair::read_from_file(env("AUTH_KEYPAIR_PATH")).unwrap(),
            );
            let _searcher_client =
                get_searcher_client(env("BLOCK_ENGINE_URL").as_str(), &auth)
                    .await
                    .expect("makes searcher client");
            return Err("Unimplemented".into());
        }
        Command::Seller {} => {
            info!("Running seller");
            seller::run_seller().await?;
        }
        Command::BenchPortal {} => {
            info!("Benching portal connection");
            bench::bench_pump_portal_connection().await?;
        }
        Command::BenchPump {} => {
            info!("Benching pump connection");
            bench::bench_pump_connection().await?;
        }
        Command::SnipePortal { lamports } => {
            info!("Sniping portal with {} lamports", lamports);
            snipe_portal::snipe_portal(lamports).await?;
        }
        Command::SnipePump { lamports } => {
            info!("Sniping pump with {} lamports", lamports);
            snipe::snipe_pump(lamports).await?;
        }
        Command::Analyze { wallet_path } => {
            let keypair = Keypair::read_from_file(wallet_path)
                .expect("Failed to read wallet");
            let rpc_client =
                Arc::new(RpcClient::new(env("RPC_URL").to_string()));
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

            let token_accounts = rpc_client
                .get_token_accounts_by_owner(
                    &keypair.pubkey(),
                    TokenAccountsFilter::ProgramId(Pubkey::from_str(
                        TOKEN_PROGRAM,
                    )?),
                )
                .await?;

            let results = token_accounts.iter().map(|token_account| {
                let token_account = token_account.clone();
                let holding =
                    parse_holding(token_account).expect("parse holding");
                let rpc_client = rpc_client.clone();
                let sniper_signatures = sniper_signatures.clone();
                let sem = semaphore.clone();
                let holding = holding.clone();
                tokio::spawn(async move {
                    // Acquire a permit
                    let _permit = sem.acquire().await.unwrap();

                    let result = async {
                        let token_transactions = rpc_client
                            .get_signatures_for_address(&holding.mint)
                            .await
                            .expect("get signatures");

                        if token_transactions.is_empty() {
                            println!(
                                "No transactions found for {}",
                                holding.mint
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

                        let txs_sniped = token_transactions
                            .iter()
                            .filter(|sig| {
                                sniper_signatures
                                    .contains(&sig.signature.to_string())
                            })
                            .map(|tx| {
                                let rpc_client = rpc_client.clone();
                                async move {
                                    rpc_client
                                    .get_transaction_with_config(
                                        &Signature::from_str(
                                            &tx.signature,
                                        )
                                        .expect("signature"),
                                        RpcTransactionConfig {
                                            encoding: Some(
                                                UiTransactionEncoding::Json,
                                            ),
                                            commitment: None,
                                            max_supported_transaction_version:
                                                Some(0),
                                        },
                                    )
                                    .await
                                    .expect("get transaction")
                                }
                            })
                            .collect::<Vec<_>>();
                        if txs_sniped.is_empty() {
                            return u64::MAX;
                        }
                        let txs_sniped = join_all(txs_sniped).await;
                        let tx_sniped = txs_sniped
                            .iter()
                            .min_by_key(|tx| tx.slot)
                            .unwrap();

                        let json_tx =
                            serde_json::to_value(&first_tx).expect("to json");
                        let is_mint_tx = json_tx["transaction"]["message"]
                            ["accountKeys"]
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
                                "{}: in {}, created: {}, sniped: {}",
                                holding.mint,
                                slots_difference,
                                first_tx.slot,
                                tx_sniped.slot
                            );
                            slots_difference
                        } else {
                            println!("No mint tx found for {}", holding.mint);
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
                println!("Total tokens analyzed: {}", valid_results.len());
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
        Command::PumpService { lamports } => {
            pump_service::run_pump_service(lamports).await?;
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
                pump::get_tokens_held_pump(&keypair.pubkey()).await?;
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

            let token_accounts = rpc_client
                .get_token_accounts_by_owner(
                    &keypair.pubkey(),
                    TokenAccountsFilter::ProgramId(Pubkey::from_str(
                        TOKEN_PROGRAM,
                    )?),
                )
                .await?;
            for token_account in token_accounts {
                let holding =
                    parse_holding(token_account).expect("parse holding");
                let mint = holding.mint;
                let pump_accounts = pump::mint_to_pump_accounts(&mint);
                if holding.amount > 0 {
                    let mint = holding.mint;
                    info!("Selling {} of {}", holding.amount, mint);
                    pump::sell_pump_token(
                        &keypair,
                        rpc_client.get_latest_blockhash().await?,
                        pump_accounts,
                        holding.amount,
                        &mut searcher_client,
                        50_000, // tip
                    )
                    .await?;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
            }
        }
        Command::BuyPumpToken { mint: _ } => {
            return Err("Unimplemented".into());
        }
    }

    Ok(())
}
