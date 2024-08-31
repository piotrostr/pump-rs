use std::sync::Arc;

use futures::future::join_all;
use log::info;
use solana_client::nonblocking::rpc_client::RpcClient;
// use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::{EncodableKey, Signer};
use solana_sdk::transaction::Transaction;

use crate::jito::send_out_bundle_to_all_regions;
use crate::util::get_jito_tip_pubkey;

pub struct WalletManager {
    pub owner: Keypair,
    pub rpc_client: Arc<RpcClient>,
    pub wallets: Vec<Keypair>,
    pub wallet_directory: String,
}

impl WalletManager {
    pub fn new(
        rpc_client: Arc<RpcClient>,
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

        let tx = Transaction::new_signed_with_payer(
            &instructions,
            Some(&self.owner.pubkey()),
            &[&self.owner],
            self.rpc_client.get_latest_blockhash().await?,
        );

        send_out_bundle_to_all_regions(&[tx]).await?;

        Ok(())
    }

    // pub async fn snipe(
    //     &self,
    //     token_mint: Pubkey,
    // ) -> Result<(), Box<dyn std::error::Error>> {
    //     for wallet in self.wallets.iter() {}

    //     Ok(())
    // }
}
