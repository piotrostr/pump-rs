use std::{collections::HashMap, error::Error};

use rand::seq::SliceRandom;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{pubkey::Pubkey, signature::Keypair, signer::Signer};
use spl_associated_token_account::get_associated_token_address;
use tracing::error;

use crate::{
    pump::{
        buy_pump_token, get_bonding_curve, get_token_amount,
        mint_to_pump_accounts, sell_pump_token,
    },
    util::env,
    wallet::{wait_balance, wait_token_balance, WalletManager},
};

/// buy_ratio 0-100, buys per 100 transactions
#[derive(Debug, Default, Clone, Copy)]
pub struct VolumeConfig {
    pub buy_ratio: u8,
    pub lamports: u64,
    pub mint: Pubkey,
    pub tip: u64,
}

#[derive(Debug, Default)]
pub struct Balances {
    pub lamports: u64,
    pub token_amounts: HashMap<Pubkey, u64>,
}

pub struct Volume {
    pub wallet_manager: WalletManager,
    pub config: VolumeConfig,
    pub wallets: HashMap<Pubkey, Balances>,
    pub queue: Vec<bool>,
}

pub async fn init(
    config: VolumeConfig,
    wallet_manager: WalletManager,
) -> Result<Volume, Box<dyn Error>> {
    // initialize the queue with ratio buy_ratio
    let mut queue = vec![false; 1000];
    queue[0..config.buy_ratio as usize]
        .iter_mut()
        .for_each(|x| *x = true);
    queue.shuffle(&mut rand::thread_rng());
    let mut wallets = HashMap::new();
    wallet_manager.fund_idempotent(config.lamports).await?;
    let lamport_balances = wallet_manager.balances().await?;
    for (pubkey, balance) in lamport_balances.iter() {
        wallets.entry(*pubkey).or_insert_with(|| Balances {
            lamports: *balance,
            token_amounts: HashMap::new(),
        });
    }
    Ok(Volume {
        queue,
        config,
        wallets,
        wallet_manager,
    })
}

impl Volume {
    pub async fn get_wallet_with_balance(
        &self,
    ) -> Result<Option<&Keypair>, Box<dyn Error>> {
        for (pubkey, balance) in &self.wallets {
            let token_balance =
                balance.token_amounts.get(&self.config.mint).unwrap_or(&0);
            if *token_balance > 0 {
                if let Some(keypair) =
                    self.wallet_manager.get_wallet_by_pubkey(pubkey)
                {
                    return Ok(Some(keypair));
                }
            }
        }
        Ok(None)
    }

    pub async fn process(
        &mut self,
        wallet_manager: &mut WalletManager,
    ) -> Result<(), Box<dyn Error>> {
        let rpc_client = RpcClient::new(env("RPC_URL"));
        let latest_blockhash = rpc_client.get_latest_blockhash().await?;
        let pump_accounts = mint_to_pump_accounts(&self.config.mint);
        if let Some(is_buy) = self.queue.pop() {
            match is_buy {
                true => {
                    // Buy operation
                    let fresh_wallet = wallet_manager.get_wallet();
                    let bonding_curve = get_bonding_curve(
                        &rpc_client,
                        pump_accounts.bonding_curve,
                    )
                    .await?;
                    let token_amount = get_token_amount(
                        bonding_curve.virtual_sol_reserves,
                        bonding_curve.virtual_token_reserves,
                        Some(bonding_curve.real_token_reserves),
                        self.config.lamports,
                    )?;
                    buy_pump_token(
                        fresh_wallet,
                        latest_blockhash,
                        pump_accounts,
                        token_amount,
                        self.config.lamports,
                        self.config.tip,
                    )
                    .await?;

                    let ata = spl_associated_token_account::get_associated_token_address(
                        &fresh_wallet.pubkey(),
                        &pump_accounts.mint,
                    );

                    wait_token_balance(&rpc_client, &ata, token_amount)
                        .await?;

                    self.wallets
                        .get_mut(&fresh_wallet.pubkey())
                        .unwrap()
                        .token_amounts
                        .insert(self.config.mint, token_amount);
                }
                false => {
                    // Sell operation
                    if let Some(wallet_with_balance) =
                        self.get_wallet_with_balance().await?
                    {
                        let token_amount = wallet_manager
                            .rpc_client
                            .get_token_account_balance(
                                &get_associated_token_address(
                                    &wallet_with_balance.pubkey(),
                                    &self.config.mint,
                                ),
                            )
                            .await?
                            .amount
                            .parse::<u64>()?;
                        if token_amount > 0 {
                            sell_pump_token(
                                wallet_with_balance,
                                wallet_manager
                                    .rpc_client
                                    .get_latest_blockhash()
                                    .await?,
                                mint_to_pump_accounts(&self.config.mint),
                                token_amount,
                                self.config.tip,
                            )
                            .await?;

                            let ata = spl_associated_token_account::get_associated_token_address(
                                &wallet_with_balance.pubkey(),
                                &pump_accounts.mint,
                            );

                            wait_token_balance(&rpc_client, &ata, 0).await?;

                            self.wallets
                                .get_mut(&wallet_with_balance.pubkey())
                                .unwrap()
                                .token_amounts
                                .insert(self.config.mint, 0);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn run(
        &mut self,
        wallet_manager: &mut WalletManager,
    ) -> Result<(), Box<dyn Error>> {
        while !self.queue.is_empty() {
            match self.process(wallet_manager).await {
                Ok(_) => {}
                Err(e) => {
                    error!("Error: {:?}", e);
                }
            };
        }
        Ok(())
    }
}
