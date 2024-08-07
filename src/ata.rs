use log::{info, warn};
use solana_account_decoder::{
    parse_account_data::ParsedAccount, UiAccountData,
};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_request::TokenAccountsFilter;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::Transaction;
use spl_token::instruction::{burn, close_account};
use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;

use crate::constants::TOKEN_PROGRAM;

/// this is dangerous, be careful
pub async fn close_all_atas(
    rpc_client: Arc<RpcClient>,
    keypair: &Keypair,
    burn_close: bool,
) -> Result<(), Box<dyn Error>> {
    if !burn_close {
        info!("This will close all ATAs with 0 balance");
    } else {
        warn!("This will burn-close all ATAs, waiting for 5 seconds, sure?");
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
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
                info!("Closing ATA: {}", ata.pubkey);
                let rpc_client = rpc_client.clone();
                let tx = Transaction::new_signed_with_payer(
                    &[close_account(
                        &Pubkey::from_str(TOKEN_PROGRAM)?,
                        &Pubkey::from_str(&ata.pubkey)?,
                        &owner,
                        &owner,
                        &[&owner],
                    )?],
                    Some(&owner),
                    &[keypair],
                    rpc_client.get_latest_blockhash().await?,
                );
                let rpc_client = rpc_client.clone();
                tokio::spawn(async move {
                    rpc_client.send_transaction(&tx).await.unwrap();
                });
            } else if burn_close {
                info!("Burn-closing: {}", ata.pubkey);
                let mint = Pubkey::from_str(
                    parsed["info"]["mint"].as_str().unwrap(),
                )?;
                let tx = Transaction::new_signed_with_payer(
                    &[
                        burn(
                            &Pubkey::from_str(TOKEN_PROGRAM)?,
                            &Pubkey::from_str(&ata.pubkey)?,
                            &mint,
                            &owner,
                            &[&owner],
                            amount_str.parse::<u64>()?,
                        )?,
                        close_account(
                            &Pubkey::from_str(TOKEN_PROGRAM)?,
                            &Pubkey::from_str(&ata.pubkey)?,
                            &owner,
                            &owner,
                            &[&owner],
                        )?,
                    ],
                    Some(&owner),
                    &[keypair],
                    rpc_client.get_latest_blockhash().await?,
                );
                let rpc_client = rpc_client.clone();
                tokio::spawn(async move {
                    rpc_client.send_transaction(&tx).await.unwrap();
                });
            }
            tokio::time::sleep(std::time::Duration::from_millis(350)).await;
        }
    }

    Ok(())
}
