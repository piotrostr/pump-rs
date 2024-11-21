use jito_searcher_client::send_bundle_no_wait;
use log::{info, warn};
use solana_account_decoder::{
    parse_account_data::ParsedAccount, UiAccountData,
};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_request::TokenAccountsFilter;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_sdk::system_instruction::transfer;
use solana_sdk::transaction::{Transaction, VersionedTransaction};
use spl_token::instruction::close_account;
use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;

use crate::constants::TOKEN_PROGRAM;
use crate::jito::SearcherClient;
use crate::util::get_jito_tip_pubkey;

/// this is dangerous, be careful
pub async fn close_all_atas(
    rpc_client: Arc<RpcClient>,
    keypair: &Keypair,
    burn_close: bool,
    searcher_client: &mut SearcherClient,
) -> Result<(), Box<dyn Error>> {
    let tip = 5_000; // super low
    if !burn_close {
        info!("This will close all ATAs with 0 balance");
    } else {
        warn!("This will burn-close all ATAs, waiting for 5 seconds, sure?");
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }
    let atas = rpc_client
        .get_token_accounts_by_owner(
            &keypair.pubkey(),
            TokenAccountsFilter::ProgramId(Pubkey::from_str(TOKEN_PROGRAM)?),
        )
        .await?;
    info!("Total ATAs: {}", atas.len());
    let owner = keypair.pubkey();
    for ata in atas {
        if let UiAccountData::Json(ParsedAccount {
            program: _,
            parsed,
            space: _,
        }) = ata.account.data
        {
            let amount_str = parsed["info"]["tokenAmount"]["amount"]
                .as_str()
                .expect("amount");
            if amount_str == "0" {
                info!("{}", ata.pubkey);
                let rpc_client = rpc_client.clone();
                let tx = VersionedTransaction::from(
                    Transaction::new_signed_with_payer(
                        &[
                            close_account(
                                &Pubkey::from_str(TOKEN_PROGRAM)?,
                                &Pubkey::from_str(&ata.pubkey)?,
                                &owner,
                                &owner,
                                &[&owner],
                            )?,
                            transfer(&owner, &get_jito_tip_pubkey(), tip),
                        ],
                        Some(&owner),
                        &[keypair],
                        rpc_client.get_latest_blockhash().await?,
                    ),
                );
                // rpc_client.send_transaction(&tx).await?;
                send_bundle_no_wait(&[tx], searcher_client).await?;
            } else if burn_close {
                info!("Burn-closing: {}", ata.pubkey);
                // don't do that no mo
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    }

    Ok(())
}
