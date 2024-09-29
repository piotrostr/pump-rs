use crate::constants::SLOT_CHECKER_MAINNET;
use crate::util::env;
use futures_util::StreamExt;
use solana_client::nonblocking::pubsub_client::PubsubClient;
use solana_sdk::hash::Hash;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::Transaction;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::debug;

pub fn update_slot(current_slot: Arc<RwLock<u64>>) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let pubsub_client = PubsubClient::new(&env("WS_URL"))
                .await
                .expect("pubsub client");
            let (mut stream, unsub) = pubsub_client
                .slot_subscribe()
                .await
                .expect("slot subscribe");
            while let Some(slot_info) = stream.next().await {
                let mut current_slot = current_slot.write().await;
                *current_slot = slot_info.slot;
                debug!("Updated slot: {}", current_slot);
            }
            unsub().await;
            // wait for a second before reconnecting
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
}

pub fn make_deadline_tx(
    deadline: u64,
    latest_blockhash: Hash,
    keypair: &Keypair,
) -> Transaction {
    Transaction::new_signed_with_payer(
        &[make_deadline_ix(deadline)],
        Some(&keypair.pubkey()),
        &[keypair],
        latest_blockhash,
    )
}

pub fn make_deadline_ix(deadline: u64) -> Instruction {
    Instruction::new_with_bytes(
        Pubkey::from_str(SLOT_CHECKER_MAINNET).expect("pubkey"),
        &deadline.to_le_bytes(),
        vec![],
    )
}
