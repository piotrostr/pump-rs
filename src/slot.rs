use crate::util::env;
use futures_util::StreamExt;
use solana_client::nonblocking::pubsub_client::PubsubClient;
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
