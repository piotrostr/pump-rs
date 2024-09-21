use std::sync::Arc;

use futures::future::join_all;
use jito_searcher_client::send_bundle_no_wait;
use log::info;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::{EncodableKey, Signer};
use solana_sdk::transaction::{Transaction, VersionedTransaction};
use tokio::sync::RwLock;

use crate::jito::{make_searcher_client, SearcherClient};
use crate::util::{env, get_jito_tip_pubkey};

pub struct WalletManager {
    pub owner: Keypair,
    pub rpc_client: Arc<RpcClient>,
    pub searcher_client: Arc<RwLock<SearcherClient>>,
    pub wallets: Vec<Keypair>,
    pub wallet_directory: String,
}

pub async fn make_manager(
) -> Result<WalletManager, Box<dyn std::error::Error>> {
    let searcher_client = make_searcher_client().await?;
    let owner = Keypair::read_from_file(env("FUND_KEYPAIR_PATH"))
        .expect("read wallet");
    let rpc_client = Arc::new(RpcClient::new(env("RPC_URL").to_string()));
    Ok(WalletManager::new(
        rpc_client.clone(),
        Arc::new(RwLock::new(searcher_client)),
        Some(env("WALLET_DIRECTORY").to_string()),
        owner,
    ))
}

impl WalletManager {
    pub fn new(
        rpc_client: Arc<RpcClient>,
        searcher_client: Arc<RwLock<SearcherClient>>,
        wallet_directory: Option<String>,
        owner: Keypair,
    ) -> Self {
        let wallet_directory =
            wallet_directory.unwrap_or_else(|| "./wallets".to_string());
        let mut wallets = Vec::new();
        let avb = std::fs::read_dir(&wallet_directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().unwrap() == "json")
            .collect::<Vec<_>>();
        for wallet in avb {
            let wallet = Keypair::read_from_file(wallet).unwrap();
            wallets.push(wallet);
        }

        info!("Read in {} wallets", wallets.len());

        Self {
            owner,
            rpc_client,
            searcher_client,
            wallets,
            wallet_directory,
        }
    }

    pub fn create_wallets(
        &mut self,
        count: usize,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let wallets = (0..count)
            .map(|_| {
                let wallet = Keypair::new();
                let path = format!(
                    "{}/{}.json",
                    self.wallet_directory,
                    wallet.pubkey()
                );
                wallet.write_to_file(&path).unwrap();
                wallet
            })
            .collect::<Vec<_>>();

        info!("Created {} wallets", wallets.len());

        self.wallets.extend(wallets);

        Ok(())
    }

    pub async fn balances(&self) -> Result<(), Box<dyn std::error::Error>> {
        let balances = self
            .wallets
            .iter()
            .map(|wallet| {
                let rpc_client = self.rpc_client.clone();
                async move {
                    let balance = rpc_client
                        .get_balance(&wallet.pubkey())
                        .await
                        .unwrap();
                    (wallet.pubkey(), balance)
                }
            })
            .collect::<Vec<_>>();

        let balances = join_all(balances).await;

        info!("Wallet balances: {:#?}", balances);
        Ok(())
    }

    pub async fn fund(
        &self,
        amount: u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let owner_balance = self
            .rpc_client
            .get_balance(&self.owner.pubkey())
            .await
            .unwrap();
        let wallets_count = self.wallets.len() as u64;
        let total_amount = amount * wallets_count;
        if owner_balance < total_amount {
            return Err("Insufficient funds".into());
        }

        let mut instructions = self
            .wallets
            .iter()
            .map(|wallet| {
                solana_sdk::system_instruction::transfer(
                    &self.owner.pubkey(),
                    &wallet.pubkey(),
                    amount,
                )
            })
            .collect::<Vec<_>>();

        instructions.push(solana_sdk::system_instruction::transfer(
            &self.owner.pubkey(),
            &get_jito_tip_pubkey(),
            10_000,
        ));

        let tx =
            VersionedTransaction::from(Transaction::new_signed_with_payer(
                &instructions,
                Some(&self.owner.pubkey()),
                &[&self.owner],
                self.rpc_client.get_latest_blockhash().await?,
            ));

        let mut searcher_client = self.searcher_client.write().await;
        send_bundle_no_wait(&[tx], &mut searcher_client).await?;

        Ok(())
    }

    pub async fn drain(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut transfer_instructions = Vec::new();
        let mut total_drained = 0;

        for wallet in &self.wallets {
            let balance =
                self.rpc_client.get_balance(&wallet.pubkey()).await?;
            // Ensure there's enough balance to cover the fee
            transfer_instructions.push((
                solana_sdk::system_instruction::transfer(
                    &wallet.pubkey(),
                    &self.owner.pubkey(),
                    balance,
                ),
                wallet,
            ));
            total_drained += balance;
        }

        if transfer_instructions.is_empty() {
            info!("No wallets with sufficient balance to drain.");
            return Ok(());
        }

        let mut transactions = Vec::new();
        let recent_blockhash = self.rpc_client.get_latest_blockhash().await?;

        for chunk in transfer_instructions.chunks(5) {
            let mut instructions: Vec<solana_sdk::instruction::Instruction> =
                chunk
                    .iter()
                    .map(|(instruction, _)| instruction.clone())
                    .collect();

            // Add Jito tip for each transaction
            instructions.push(solana_sdk::system_instruction::transfer(
                &self.owner.pubkey(),
                &get_jito_tip_pubkey(),
                10_000,
            ));

            let mut signers: Vec<&Keypair> =
                chunk.iter().map(|(_, wallet)| *wallet).collect();
            signers.push(&self.owner);

            let tx = VersionedTransaction::from(
                Transaction::new_signed_with_payer(
                    &instructions,
                    Some(&self.owner.pubkey()),
                    &signers,
                    recent_blockhash,
                ),
            );

            info!("Transaction: {:#?}", tx);
            // simulate tx
            let _ = self
                .rpc_client
                .simulate_transaction(&tx)
                .await
                .expect("simulate tx");

            transactions.push(tx);
        }

        let mut searcher_client = self.searcher_client.write().await;
        send_bundle_no_wait(&transactions, &mut searcher_client).await?;

        info!(
            "Drained {} lamports from {} wallets in {} transactions",
            total_drained,
            &self.wallets.len(),
            transactions.len()
        );

        Ok(())
    }
}
