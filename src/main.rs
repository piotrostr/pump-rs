use chrono::Local;
use env_logger::Builder;
use jito_searcher_client::get_searcher_client;
use log::LevelFilter;
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
use tokio::sync::Mutex;

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
        Command::CloseTokenAccounts { wallet_path } => {
            let keypair =
                Keypair::read_from_file(wallet_path).expect("read wallet");
            info!("Wallet: {}", keypair.pubkey());
            let rpc_client =
                Arc::new(RpcClient::new(env("RPC_URL").to_string()));
            ata::close_all_atas(rpc_client, &keypair).await?;
        }
        Command::PumpService {} => {
            pump_service::run_pump_service().await?;
        }
        Command::SellPump { mint } => {
            let keypair =
                Keypair::read_from_file("wtf.json").expect("read wallet");
            let rpc_client = RpcClient::new(env("RPC_URL").to_string());
            let ata =
                spl_associated_token_account::get_associated_token_address(
                    &keypair.pubkey(),
                    &Pubkey::from_str(&mint)?,
                );

            let actual_balance = rpc_client
                .get_token_account_balance(&ata)
                .await?
                .amount
                .parse::<u64>()?;

            let pump_accounts =
                pump::mint_to_pump_accounts(&Pubkey::from_str(&mint)?)
                    .await?;

            pump::sell_pump_token(
                &keypair,
                &rpc_client,
                pump_accounts,
                actual_balance,
            )
            .await?;
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
                    let actual_balance = rpc_client
                        .get_token_account_balance(&ata)
                        .await?
                        .amount
                        .parse::<u64>()?;
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
                        )
                        .await?;
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
