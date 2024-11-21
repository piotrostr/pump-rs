use futures::future::join_all;
use solana_client::rpc_config::RpcTransactionConfig;
use solana_client::rpc_request::TokenAccountsFilter;
use solana_transaction_status::UiTransactionEncoding;
use tokio::sync::Semaphore;

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use tokio::time::Duration;

use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signature};
use solana_sdk::signer::{EncodableKey, Signer};

use crate::constants::{PUMP_FUN_MINT_AUTHORITY, TOKEN_PROGRAM};
use crate::util::{env, parse_holding};

pub async fn run_analysis(
    wallet_path: Option<String>,
    address: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let pubkey = if let Some(wallet_path) = wallet_path {
        let keypair = Keypair::read_from_file(wallet_path)
            .expect("Failed to read wallet");
        keypair.pubkey()
    } else if let Some(address) = address {
        Pubkey::from_str(&address).expect("parse pubkey")
    } else {
        panic!("Either wallet path or address must be provided");
    };
    let rpc_client = Arc::new(RpcClient::new(env("RPC_URL").to_string()));
    let sniper_signatures =
        rpc_client.get_signatures_for_address(&pubkey).await?;
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
            &pubkey,
            TokenAccountsFilter::ProgramId(Pubkey::from_str(TOKEN_PROGRAM)?),
        )
        .await?;

    let results = token_accounts.iter().map(|token_account| {
        let token_account = token_account.clone();
        let holding = parse_holding(token_account).expect("parse holding");
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
                    println!("No transactions found for {}", holding.mint);
                    return u64::MAX;
                }

                let first_tx_sig = token_transactions.last().unwrap();
                let first_tx = rpc_client
                    .get_transaction_with_config(
                        &Signature::from_str(&first_tx_sig.signature)
                            .expect("signature"),
                        RpcTransactionConfig {
                            encoding: Some(UiTransactionEncoding::Json),
                            commitment: None,
                            max_supported_transaction_version: Some(0),
                        },
                    )
                    .await
                    .expect("get transaction");

                let txs_sniped = token_transactions
                    .iter()
                    .filter(|sig| {
                        sniper_signatures.contains(&sig.signature.to_string())
                    })
                    .map(|tx| {
                        let rpc_client = rpc_client.clone();
                        async move {
                            rpc_client
                                .get_transaction_with_config(
                                    &Signature::from_str(&tx.signature)
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
                let tx_sniped =
                    txs_sniped.iter().min_by_key(|tx| tx.slot).unwrap();

                let json_tx =
                    serde_json::to_value(&first_tx).expect("to json");
                let is_mint_tx = json_tx["transaction"]["message"]
                    ["accountKeys"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|key| {
                        key.as_str().unwrap() == PUMP_FUN_MINT_AUTHORITY
                    });

                if is_mint_tx {
                    let slots_difference = tx_sniped.slot - first_tx.slot;
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

            // sometimes it messes up, can skip those entries
            if result < 200 {
                result
            } else {
                u64::MAX
            }
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
        let average_slots = total_slots as f64 / valid_results.len() as f64;
        println!("Total tokens analyzed: {}", valid_results.len());
        println!("Average snipe slots: {:.2}", average_slots);
    } else {
        println!("No valid results to calculate average");
    }

    Ok(())
}
