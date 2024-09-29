use solana_client::nonblocking::{
    rpc_client::RpcClient, tpu_client::TpuClient,
};
use solana_client::tpu_client::TpuClientConfig;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::{EncodableKey, Signer};
use solana_sdk::system_instruction::transfer;
use solana_sdk::transaction::Transaction;
use std::error::Error;
use std::sync::Arc;

use crate::util::env;

pub async fn send_tx_tpu() -> Result<(), Box<dyn Error>> {
    let rpc_url = env("RPC_URL");
    let rpc_client = Arc::new(RpcClient::new(rpc_url.clone()));
    let funder = Keypair::read_from_file(env("FUND_KEYPAIR_PATH")).unwrap();
    println!("Funder: {}", funder.pubkey());
    let tpu_client = TpuClient::new(
        "client",
        rpc_client.clone(),
        &rpc_url.replace("http", "ws"),
        TpuClientConfig::default(),
    )
    .await
    .unwrap();

    let tx = Transaction::new_signed_with_payer(
        &[transfer(&funder.pubkey(), &funder.pubkey(), 10000)],
        Some(&funder.pubkey()),
        &[&funder],
        rpc_client.get_latest_blockhash().await?,
    );

    println!("current slot: {}", rpc_client.get_slot().await?);

    tpu_client.try_send_transaction(&tx).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn send_tx_tpu() {
        dotenv::dotenv().ok();
        super::send_tx_tpu().await.unwrap();
    }
}
