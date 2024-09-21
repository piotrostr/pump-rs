use crate::jito::{send_out_bundle_to_all_regions, SearcherClient};
use futures_util::stream::StreamExt;
use jito_protos::searcher::SubscribeBundleResultsRequest;
use jito_searcher_client::{
    send_bundle_no_wait, send_bundle_with_confirmation,
};
use log::{debug, error, info, warn};
use solana_account_decoder::UiAccountEncoding;
use solana_client::nonblocking::pubsub_client::PubsubClient;
use solana_client::rpc_config::{
    RpcAccountInfoConfig, RpcSendTransactionConfig, RpcTransactionConfig,
    RpcTransactionLogsConfig, RpcTransactionLogsFilter,
};
use solana_sdk::hash::Hash;
use solana_sdk::system_instruction::transfer;
use solana_sdk::transaction::{Transaction, VersionedTransaction};
use std::error::Error;

use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};

use borsh::{BorshDeserialize, BorshSerialize};

use serde::{Deserialize, Serialize};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signature};
use solana_sdk::signer::Signer;
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, EncodedTransaction, UiMessage,
    UiParsedMessage, UiTransactionEncoding,
};

use crate::constants::{
    ASSOCIATED_TOKEN_PROGRAM, EVENT_AUTHORITY, PUMP_BUY_METHOD,
    PUMP_FEE_ADDRESS, PUMP_FUN_MINT_AUTHORITY, PUMP_FUN_PROGRAM,
    PUMP_GLOBAL_ADDRESS, PUMP_SELL_METHOD, RENT_PROGRAM, SYSTEM_PROGRAM_ID,
    TOKEN_PROGRAM,
};
use crate::util::{
    env, get_jito_tip_pubkey, make_compute_budget_ixs, pubkey_to_string,
    string_to_pubkey, /*string_to_u64,*/
};

#[derive(BorshSerialize)]
pub struct PumpFunSwapInstructionData {
    pub method_id: [u8; 8],
    pub token_amount: u64,
    pub lamports: u64,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct BondingCurveLayout {
    pub blob1: u64,
    pub virtual_token_reserves: u64,
    pub virtual_sol_reserves: u64,
    pub real_token_reserves: u64,
    pub real_sol_reserves: u64,
    pub blob4: u64,
    pub complete: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PumpTokenData {
    pub address: String,
    pub balance: u64,
    pub image_uri: String,
    pub market_cap: f64,
    pub mint: String,
    pub name: String,
    pub symbol: String,
    pub value: f64,
}

impl BondingCurveLayout {
    pub const LEN: usize = 8 + 8 + 8 + 8 + 8 + 8 + 1;

    pub fn parse(data: &[u8]) -> Result<Self, std::io::Error> {
        Self::try_from_slice(data)
    }
}

pub async fn get_slot_created(
    rpc_client: &RpcClient,
    mint: &Pubkey,
) -> Result<u64, Box<dyn std::error::Error>> {
    let token_transactions =
        rpc_client.get_signatures_for_address(mint).await?;

    if token_transactions.is_empty() {
        warn!("No transactions found for mint: {}", mint);
        return Ok(0);
    }

    let first_tx_sig = token_transactions.last().unwrap();
    let first_tx = rpc_client
        .get_transaction_with_config(
            &Signature::from_str(&first_tx_sig.signature).expect("signature"),
            RpcTransactionConfig {
                encoding: Some(UiTransactionEncoding::Json),
                commitment: None,
                max_supported_transaction_version: Some(0),
            },
        )
        .await?;

    Ok(first_tx.slot)
}

pub fn mint_to_pump_accounts(mint: &Pubkey) -> PumpAccounts {
    // Derive the bonding curve address
    let (bonding_curve, _) = Pubkey::find_program_address(
        &[b"bonding-curve", mint.as_ref()],
        &Pubkey::from_str(PUMP_FUN_PROGRAM).unwrap(),
    );

    // Derive the associated bonding curve address
    let associated_bonding_curve =
        spl_associated_token_account::get_associated_token_address(
            &bonding_curve,
            mint,
        );

    PumpAccounts {
        mint: *mint,
        bonding_curve,
        associated_bonding_curve,
        dev: Pubkey::default(),
        metadata: Pubkey::default(),
    }
}

pub fn subscribe_to_pump(slot: Arc<RwLock<u64>>) -> JoinHandle<()> {
    let slot = slot.clone();
    tokio::spawn(async move {
        loop {
            let pubsub_client = PubsubClient::new(&env("WS_URL"))
                .await
                .expect("pubsub client");
            let (mut stream, unsub) = pubsub_client
                .logs_subscribe(
                    RpcTransactionLogsFilter::Mentions(vec![
                        PUMP_FUN_MINT_AUTHORITY.to_string(),
                    ]),
                    RpcTransactionLogsConfig {
                        commitment: Some(CommitmentConfig::confirmed()),
                    },
                )
                .await
                .expect("logs subscribe");
            info!("Subscribed to pump logs");
            while let Some(data) = stream.next().await {
                tracing::info!(
                    "{}: {}",
                    slot.read().await,
                    data.value.signature
                );
            }
            unsub().await;
        }
    })
}

#[timed::timed(duration(printer = "info!"))]
pub async fn get_tokens_held_pump(
    owner: &Pubkey,
) -> Result<Vec<PumpTokenData>, Box<dyn Error>> {
    let url = "https://frontend-api.pump.fun/balances/{}?limit=6666&offset=0&minBalance=0";
    let url = url.replace("{}", &owner.to_string());
    Ok(reqwest::get(&url)
        .await?
        .json::<Vec<PumpTokenData>>()
        .await?)
}

pub async fn get_bonding_curve(
    rpc_client: &RpcClient,
    bonding_curve_pubkey: Pubkey,
) -> Result<BondingCurveLayout, Box<dyn Error>> {
    const MAX_RETRIES: u32 = 5;
    const INITIAL_DELAY_MS: u64 = 200;
    let mut retries = 0;
    let mut delay = Duration::from_millis(INITIAL_DELAY_MS);

    loop {
        match rpc_client
            .get_account_with_config(
                &bonding_curve_pubkey,
                RpcAccountInfoConfig {
                    encoding: Some(UiAccountEncoding::Base64),
                    commitment: Some(CommitmentConfig::processed()),
                    data_slice: None,
                    min_context_slot: None,
                },
            )
            .await
        {
            Ok(res) => {
                if let Some(account) = res.value {
                    // Convert Vec<u8> to [u8; 49]
                    let data_length = account.data.len();
                    let data: [u8; 49] =
                        account.data.try_into().map_err(|_| {
                            format!("Invalid data length: {}", data_length)
                        })?;

                    debug!("Raw bytes: {:?}", data);

                    let layout = BondingCurveLayout {
                        blob1: u64::from_le_bytes(data[0..8].try_into()?),
                        virtual_token_reserves: u64::from_le_bytes(
                            data[8..16].try_into()?,
                        ),
                        virtual_sol_reserves: u64::from_le_bytes(
                            data[16..24].try_into()?,
                        ),
                        real_token_reserves: u64::from_le_bytes(
                            data[24..32].try_into()?,
                        ),
                        real_sol_reserves: u64::from_le_bytes(
                            data[32..40].try_into()?,
                        ),
                        blob4: u64::from_le_bytes(data[40..48].try_into()?),
                        complete: data[48] != 0,
                    };

                    debug!("Parsed BondingCurveLayout: {:?}", layout);
                    return Ok(layout);
                } else {
                    if retries >= MAX_RETRIES {
                        error!("Max retries reached. Account not found.");
                        return Err(
                            "Account not found after max retries".into()
                        );
                    }
                    warn!(
                        "Attempt {} failed: Account not found. Retrying in {:?}...",
                        retries + 1,
                        delay
                    );
                    sleep(delay).await;
                    retries += 1;
                    delay = Duration::from_millis(
                        INITIAL_DELAY_MS * 2u64.pow(retries),
                    );
                    continue;
                }
            }
            Err(e) => {
                if retries >= MAX_RETRIES {
                    error!("Max retries reached. Last error: {}", e);
                    return Err(format!(
                        "Max retries reached. Last error: {}",
                        e
                    )
                    .into());
                }
                warn!(
                    "Attempt {} failed: {}. Retrying in {:?}...",
                    retries + 1,
                    e,
                    delay
                );
                sleep(delay).await;
                retries += 1;
                delay = Duration::from_millis(
                    INITIAL_DELAY_MS * 2u64.pow(retries),
                );
            }
        }
    }
}

pub fn get_token_amount(
    virtual_sol_reserves: u64,
    virtual_token_reserves: u64,
    real_token_reserves: Option<u64>,
    lamports: u64,
) -> Result<u64, Box<dyn std::error::Error>> {
    let virtual_sol_reserves = virtual_sol_reserves as u128;
    let virtual_token_reserves = virtual_token_reserves as u128;
    let amount_in = lamports as u128;

    // Calculate reserves_product carefully to avoid overflow
    let reserves_product = virtual_sol_reserves
        .checked_mul(virtual_token_reserves)
        .ok_or("Overflow in reserves product calculation")?;

    let new_virtual_sol_reserve = virtual_sol_reserves
        .checked_add(amount_in)
        .ok_or("Overflow in new virtual SOL reserve calculation")?;

    let new_virtual_token_reserve = reserves_product
        .checked_div(new_virtual_sol_reserve)
        .ok_or("Division by zero or overflow in new virtual token reserve calculation")?
        .checked_add(1)
        .ok_or("Overflow in new virtual token reserve calculation")?;

    let amount_out = virtual_token_reserves
        .checked_sub(new_virtual_token_reserve)
        .ok_or("Underflow in amount out calculation")?;

    let final_amount_out =
        if let Some(real_token_reserves) = real_token_reserves {
            std::cmp::min(amount_out, real_token_reserves as u128)
        } else {
            amount_out
        };

    Ok(final_amount_out as u64)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PumpBuyRequest {
    #[serde(
        serialize_with = "pubkey_to_string",
        deserialize_with = "string_to_pubkey"
    )]
    pub mint: Pubkey,
    #[serde(
        serialize_with = "pubkey_to_string",
        deserialize_with = "string_to_pubkey"
    )]
    pub bonding_curve: Pubkey,
    #[serde(
        serialize_with = "pubkey_to_string",
        deserialize_with = "string_to_pubkey"
    )]
    pub associated_bonding_curve: Pubkey,
    // with typescript service, the data is passed as string
    // commenting this out since I am passing u64 from rust service now
    // #[serde(deserialize_with = "string_to_u64")]
    pub virtual_token_reserves: u64,
    // #[serde(deserialize_with = "string_to_u64")]
    pub virtual_sol_reserves: u64,

    pub slot: Option<u64>,
}

pub async fn buy_pump_token(
    wallet: &Keypair,
    rpc_client: &RpcClient,
    pump_accounts: PumpAccounts,
    lamports: u64,
    searcher_client: &mut Arc<Mutex<SearcherClient>>,
    use_jito: bool,
) -> Result<(), Box<dyn Error>> {
    let owner = wallet.pubkey();

    let bonding_curve =
        get_bonding_curve(rpc_client, pump_accounts.bonding_curve).await?;
    let token_amount = get_token_amount(
        bonding_curve.virtual_sol_reserves,
        bonding_curve.virtual_token_reserves,
        Some(bonding_curve.real_token_reserves),
        lamports,
    )?;

    // apply slippage in a stupid manner
    let token_amount = (token_amount as f64 * 0.97) as u64;

    info!("buying {}", token_amount);

    let mut ixs = _make_buy_ixs(
        owner,
        pump_accounts.mint,
        pump_accounts.bonding_curve,
        pump_accounts.associated_bonding_curve,
        token_amount,
        lamports,
    )?;

    if use_jito {
        let tip = 100_000;
        let mut searcher_client = searcher_client.lock().await;
        let latest_blockhash = rpc_client.get_latest_blockhash().await?;
        ixs.push(transfer(&owner, &get_jito_tip_pubkey(), tip));
        let tx =
            VersionedTransaction::from(Transaction::new_signed_with_payer(
                &ixs,
                Some(&owner),
                &[wallet],
                latest_blockhash,
            ));
        send_bundle_no_wait(&[tx], &mut searcher_client).await?;
    } else {
        _send_tx_standard(ixs, wallet, rpc_client, owner).await?;
    }

    // send the tx with spinner
    // let res = rpc_client
    //     .send_and_confirm_transaction_with_spinner_and_config(
    //         &transaction,
    //         CommitmentConfig::processed(),
    //         RpcSendTransactionConfig {
    //             encoding: Some(UiTransactionEncoding::Base64),
    //             skip_preflight: true,
    //             max_retries: None,
    //             preflight_commitment: None,
    //             min_context_slot: None,
    //         },
    //     )
    //     .await;
    //
    // send the transaction without spinner

    Ok(())
}

pub fn _make_buy_ixs(
    owner: Pubkey,
    mint: Pubkey,
    bonding_curve: Pubkey,
    associated_bonding_curve: Pubkey,
    token_amount: u64,
    lamports: u64,
) -> Result<Vec<Instruction>, Box<dyn Error>> {
    let mut ixs = vec![];
    let ata = spl_associated_token_account::get_associated_token_address(
        &owner, &mint,
    );

    ixs.push(
        spl_associated_token_account::instruction::create_associated_token_account_idempotent(
            &owner,
            &owner,
            &mint,
            &spl_token::id(),
        ),
    );
    ixs.push(make_pump_swap_ix(
        owner,
        mint,
        bonding_curve,
        associated_bonding_curve,
        token_amount,
        lamports,
        ata,
    )?);

    Ok(ixs)
}

async fn _send_tx_standard(
    ixs: Vec<Instruction>,
    wallet: &Keypair,
    rpc_client: &RpcClient,
    owner: Pubkey,
) -> Result<(), Box<dyn Error>> {
    let transaction =
        VersionedTransaction::from(Transaction::new_signed_with_payer(
            &ixs,
            Some(&owner),
            &[wallet],
            rpc_client.get_latest_blockhash().await?,
        ));
    let res = rpc_client
        .send_transaction_with_config(
            &transaction,
            RpcSendTransactionConfig {
                skip_preflight: true,
                min_context_slot: None,
                preflight_commitment: Some(CommitmentLevel::Processed),
                max_retries: None,
                encoding: None,
            },
        )
        .await;

    match res {
        Ok(sig) => {
            info!("Transaction sent: {}", sig);
        }
        Err(e) => {
            return Err(e.into());
        }
    }

    Ok(())
}

#[timed::timed(duration(printer = "info!"))]
pub async fn sell_pump_token(
    wallet: &Keypair,
    latest_blockhash: Hash,
    pump_accounts: PumpAccounts,
    token_amount: u64,
    _searcher_client: &mut SearcherClient,
    tip: u64,
) -> Result<(), Box<dyn Error>> {
    let owner = wallet.pubkey();

    let ata = spl_associated_token_account::get_associated_token_address(
        &owner,
        &pump_accounts.mint,
    );

    let mut ixs = vec![];
    let mut compute_budget_ixs = make_compute_budget_ixs(25_000, 272_500);
    let sell_ix = make_pump_sell_ix(owner, pump_accounts, token_amount, ata)?;
    ixs.append(&mut compute_budget_ixs);
    ixs.push(sell_ix);
    ixs.push(transfer(&owner, &get_jito_tip_pubkey(), tip));

    let tx = Transaction::new_signed_with_payer(
        &ixs,
        Some(&owner),
        &[wallet],
        latest_blockhash,
    );
    // let _versioned_tx = VersionedTransaction::from(tx.clone());

    send_out_bundle_to_all_regions(&[tx]).await?;

    Ok(())
}

/// Interact With Pump.Fun - 6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P
/// #1 - Global
/// #2 - Fee Recipient: Pump.fun Fee Account [Writable]
/// #3 - Mint
/// #4 - Bonding Curve [Writable]
/// #5 - Associated Bonding Curve [Writable]
/// #6 - Associated Token Account (ATA) [Writable]
/// #7 - User [Writable Signer Fee-Payer]
/// #8 - System Program
/// #9 - Associated Token Program
/// #10 - Token Program
/// #11 - Event Authority
/// #12 - Program: Pump.fun Program
pub fn make_pump_sell_ix(
    owner: Pubkey,
    pump_accounts: PumpAccounts,
    token_amount: u64,
    ata: Pubkey,
) -> Result<Instruction, Box<dyn Error>> {
    let accounts: [AccountMeta; 12] = [
        AccountMeta::new_readonly(
            Pubkey::from_str(PUMP_GLOBAL_ADDRESS)?,
            false,
        ),
        AccountMeta::new(Pubkey::from_str(PUMP_FEE_ADDRESS)?, false),
        AccountMeta::new_readonly(pump_accounts.mint, false),
        AccountMeta::new(pump_accounts.bonding_curve, false),
        AccountMeta::new(pump_accounts.associated_bonding_curve, false),
        AccountMeta::new(ata, false),
        AccountMeta::new(owner, true),
        AccountMeta::new_readonly(
            Pubkey::from_str(SYSTEM_PROGRAM_ID)?,
            false,
        ),
        AccountMeta::new_readonly(
            Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM)?,
            false,
        ),
        AccountMeta::new_readonly(Pubkey::from_str(TOKEN_PROGRAM)?, false),
        AccountMeta::new_readonly(Pubkey::from_str(EVENT_AUTHORITY)?, false),
        AccountMeta::new_readonly(Pubkey::from_str(PUMP_FUN_PROGRAM)?, false),
    ];

    // max slippage, careful if not using frontrun protection
    let data = PumpFunSwapInstructionData {
        method_id: PUMP_SELL_METHOD,
        token_amount,
        lamports: 0,
    };

    Ok(Instruction::new_with_borsh(
        Pubkey::from_str(PUMP_FUN_PROGRAM)?,
        &data,
        accounts.to_vec(),
    ))
}

/// Interact With Pump.Fun 6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P
/// Input Accounts
/// #1 - Global: 4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf
/// #2 - Fee Recipient: Pump.fun Fee Account [Writable]
/// #3 - Mint
/// #4 - Bonding Curve [Writable]
/// #5 - Associated Bonding Curve [Writable]
/// #6 - Associated User Account [Writable] (ATA)
/// #7 - User - owner, sender [Writable, Signer, Fee Payer]
/// #8 - System Program (11111111111111111111111111111111)
/// #9 - Token Program (TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA)
/// #10 - Rent (SysvarRent111111111111111111111111111111111)
/// #11 - Event Authority: Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1
/// #12 - Program: Pump.fun Program 6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P
pub fn make_pump_swap_ix(
    owner: Pubkey,
    mint: Pubkey,
    bonding_curve: Pubkey,
    associated_bonding_curve: Pubkey,
    token_amount: u64,
    lamports: u64,
    ata: Pubkey,
) -> Result<Instruction, Box<dyn Error>> {
    let accounts: [AccountMeta; 12] = [
        AccountMeta::new_readonly(
            Pubkey::from_str(PUMP_GLOBAL_ADDRESS)?,
            false,
        ),
        AccountMeta::new(Pubkey::from_str(PUMP_FEE_ADDRESS)?, false),
        AccountMeta::new_readonly(mint, false),
        AccountMeta::new(bonding_curve, false),
        AccountMeta::new(associated_bonding_curve, false),
        AccountMeta::new(ata, false),
        AccountMeta::new(owner, true),
        AccountMeta::new_readonly(
            Pubkey::from_str(SYSTEM_PROGRAM_ID)?,
            false,
        ),
        AccountMeta::new_readonly(Pubkey::from_str(TOKEN_PROGRAM)?, false),
        AccountMeta::new_readonly(Pubkey::from_str(RENT_PROGRAM)?, false),
        AccountMeta::new_readonly(Pubkey::from_str(EVENT_AUTHORITY)?, false),
        AccountMeta::new_readonly(Pubkey::from_str(PUMP_FUN_PROGRAM)?, false),
    ];

    let data = PumpFunSwapInstructionData {
        method_id: PUMP_BUY_METHOD,
        token_amount,
        lamports,
    };

    Ok(Instruction::new_with_borsh(
        Pubkey::from_str(PUMP_FUN_PROGRAM)?,
        &data,
        accounts.to_vec(),
    ))
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub struct PumpAccounts {
    #[serde(
        serialize_with = "pubkey_to_string",
        deserialize_with = "string_to_pubkey"
    )]
    pub mint: Pubkey,
    #[serde(
        serialize_with = "pubkey_to_string",
        deserialize_with = "string_to_pubkey"
    )]
    pub bonding_curve: Pubkey,
    #[serde(
        serialize_with = "pubkey_to_string",
        deserialize_with = "string_to_pubkey"
    )]
    pub associated_bonding_curve: Pubkey,
    #[serde(
        serialize_with = "pubkey_to_string",
        deserialize_with = "string_to_pubkey"
    )]
    pub dev: Pubkey,
    #[serde(
        serialize_with = "pubkey_to_string",
        deserialize_with = "string_to_pubkey"
    )]
    pub metadata: Pubkey,
}

pub fn parse_pump_accounts(
    tx: EncodedConfirmedTransactionWithStatusMeta,
) -> Result<PumpAccounts, Box<dyn Error>> {
    if let EncodedTransaction::Json(tx) = &tx.transaction.transaction {
        if let UiMessage::Parsed(UiParsedMessage {
            account_keys,
            instructions: _,
            recent_blockhash: _,
            address_table_lookups: _,
        }) = &tx.message
        {
            debug!("Account keys: {:?}", account_keys);
            if account_keys.len() >= 5 {
                let dev = account_keys[0].pubkey.parse()?;
                let mint = account_keys[1].pubkey.parse()?;
                let bonding_curve = account_keys[3].pubkey.parse()?;
                let associated_bonding_curve =
                    account_keys[4].pubkey.parse()?;
                let metadata = account_keys[5].pubkey.parse()?;

                Ok(PumpAccounts {
                    mint,
                    bonding_curve,
                    associated_bonding_curve,
                    dev,
                    metadata,
                })
            } else {
                Err("Not enough account keys".into())
            }
        } else {
            Err("Not a parsed transaction".into())
        }
    } else {
        Err("Not a JSON transaction".into())
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PumpTokenInfo {
    pub associated_bonding_curve: String,
    pub bonding_curve: String,
    pub complete: bool,
    pub created_timestamp: i64,
    pub creator: String,
    pub description: String,
    pub image_uri: String,
    pub inverted: bool,
    pub is_currently_live: bool,
    pub king_of_the_hill_timestamp: i64,
    pub last_reply: i64,
    pub market_cap: f64,
    pub market_id: String,
    pub metadata_uri: String,
    pub mint: String,
    pub name: String,
    pub nsfw: bool,
    pub profile_image: Option<String>,
    pub raydium_pool: String,
    pub reply_count: i32,
    pub show_name: bool,
    pub symbol: String,
    pub telegram: Option<String>,
    pub total_supply: i64,
    pub twitter: Option<String>,
    pub usd_market_cap: f64,
    pub username: Option<String>,
    pub virtual_sol_reserves: i64,
    pub virtual_token_reserves: i64,
    pub website: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IPFSMetadata {
    pub name: String,
    pub symbol: String,
    pub description: String,
    pub image: String,
    #[serde(rename = "showName")]
    pub show_name: Option<bool>,
    #[serde(rename = "createdOn")]
    pub created_on: Option<String>,
    pub twitter: Option<String>,
    pub telegram: Option<String>,
    pub website: Option<String>,
}

pub async fn fetch_metadata(
    mint: &Pubkey,
) -> Result<PumpTokenInfo, Box<dyn Error>> {
    const MAX_RETRIES: u32 = 3;
    const INITIAL_DELAY_MS: u64 = 100;

    let mut retry_count = 0;
    let mut delay_ms = INITIAL_DELAY_MS;

    loop {
        match fetch_metadata_inner(mint).await {
            Ok(metadata) => {
                info!("Metadata fetched successfully");
                return Ok(metadata);
            }
            Err(e) => {
                if retry_count >= MAX_RETRIES {
                    info!("Failed to fetch metadata after all retries");
                    return Err(e);
                }
                info!(
                    "Retry attempt {} failed: {:?}. Retrying in {} ms...",
                    retry_count + 1,
                    e,
                    delay_ms
                );
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                retry_count += 1;
                delay_ms *= 2; // Exponential backoff
            }
        }
    }
}

async fn fetch_metadata_inner(
    mint: &Pubkey,
) -> Result<PumpTokenInfo, Box<dyn Error>> {
    let url = format!("https://frontend-api.pump.fun/coins/{}", mint);
    info!("Fetching metadata from: {}", url);
    let res = reqwest::get(&url).await?;
    info!("res: {:?}", res);
    let data = res.json::<PumpTokenInfo>().await?;
    Ok(data)
}

/// send_pump_bump is idempotent, if the ata does not exist it will make a buy
/// and sell to create it, otherwise it sends a simple buy and sell ixs
/// transaction
pub async fn send_pump_bump(
    wallet: &Keypair,
    rpc_client: &RpcClient,
    mint: &Pubkey,
    searcher_client: &mut Arc<Mutex<SearcherClient>>,
    wait_for_confirmation: bool,
) -> Result<(), Box<dyn Error>> {
    let lamports = 22_800_000;
    let owner = wallet.pubkey();
    let pump_accounts = mint_to_pump_accounts(mint);
    let bonding_curve =
        get_bonding_curve(rpc_client, pump_accounts.bonding_curve).await?;
    let token_amount = get_token_amount(
        bonding_curve.virtual_sol_reserves,
        bonding_curve.virtual_token_reserves,
        Some(bonding_curve.real_token_reserves),
        lamports,
    )?;
    let token_amount = (token_amount as f64 * 0.95) as u64;

    let ata = spl_associated_token_account::get_associated_token_address(
        &owner,
        &pump_accounts.mint,
    );

    if rpc_client.get_account(&ata).await.is_err() {
        warn!("ata does not exist, creating it through buy and sell");
        buy_pump_token(
            wallet,
            rpc_client,
            pump_accounts,
            lamports,
            searcher_client,
            true,
        )
        .await?;

        let mut searcher_client = searcher_client.lock().await;
        sell_pump_token(
            wallet,
            rpc_client.get_latest_blockhash().await?,
            pump_accounts,
            token_amount,
            &mut searcher_client,
            50_000,
        )
        .await?;
        return Ok(());
    }

    let mut ixs = vec![];
    // ixs.append(&mut make_compute_budget_ixs(262500, 10000));

    ixs.push(make_pump_swap_ix(
        owner,
        pump_accounts.mint,
        pump_accounts.bonding_curve,
        pump_accounts.associated_bonding_curve,
        token_amount,
        lamports,
        ata,
    )?);

    ixs.push(make_pump_sell_ix(owner, pump_accounts, token_amount, ata)?);

    // 0.00005 sol
    let tip = 50_000;
    ixs.push(transfer(&owner, &get_jito_tip_pubkey(), tip));

    let tx = VersionedTransaction::from(Transaction::new_signed_with_payer(
        &ixs,
        Some(&owner),
        &[wallet],
        rpc_client.get_latest_blockhash().await?,
    ));

    let mut searcher_client = searcher_client.lock().await;

    if wait_for_confirmation {
        let mut bundle_results_subscription = searcher_client
            .subscribe_bundle_results(SubscribeBundleResultsRequest {})
            .await
            .expect("subscribe to bundle results")
            .into_inner();

        send_bundle_with_confirmation(
            &[tx],
            rpc_client,
            &mut searcher_client,
            &mut bundle_results_subscription,
        )
        .await?;
    } else {
        send_bundle_no_wait(&[tx], &mut searcher_client).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use jito_searcher_client::get_searcher_client;
    use solana_sdk::signer::EncodableKey;

    use crate::util::env;

    use super::*;

    #[tokio::test]
    async fn test_pump_bump() {
        dotenv::from_filename(".env").unwrap();
        let wallet = Keypair::read_from_file(env("FUND_KEYPAIR_PATH"))
            .expect("read wallet");
        let rpc_client =
            RpcClient::new("https://api.mainnet-beta.solana.com".to_string());
        let mint =
            Pubkey::from_str("8ALbiQ2aWD4V63bx6s5qtf21LA4r9uBaY2THbg9epump")
                .unwrap();
        let auth = Arc::new(
            Keypair::read_from_file(env("AUTH_KEYPAIR_PATH")).unwrap(),
        );
        let mut searcher_client = Arc::new(Mutex::new(
            get_searcher_client(env("BLOCK_ENGINE_URL").as_str(), &auth)
                .await
                .expect("makes searcher client"),
        ));

        send_pump_bump(
            &wallet,
            &rpc_client,
            &mint,
            &mut searcher_client,
            true,
        )
        .await
        .expect("send_pump_bump");
    }

    #[tokio::test]
    async fn test_fetch_metadata() {
        let metadata = fetch_metadata(
            &Pubkey::from_str("4cRkQ2dntpusYag6Zmvco8T78WxK9Jqh1eEZJox8pump")
                .expect("parse mint"),
        )
        .await
        .expect("fetch_metadata");

        assert_eq!(metadata.name, "🗿".to_string());
        assert_eq!(metadata.symbol, "🗿".to_string());
        assert_eq!(
            metadata.image_uri,
            "https://cf-ipfs.com/ipfs/QmXn5xkUMxNQ5c5Sfct8rFTq9jNi6jsSHm1yLY2nQyeSke".to_string()
        );
        assert_eq!(
            metadata.twitter,
            Some("https://x.com/thefirstgigasol".to_string())
        );
        assert_eq!(
            metadata.telegram,
            Some("https://t.me/+keptGgOKxN45YWRl".to_string())
        );
        assert_eq!(
            metadata.website,
            Some("https://thefirstgiga.com/".to_string())
        );
    }

    #[test]
    fn test_parse_pump_accounts() {
        let sample_tx =
            std::fs::read_to_string("pump_fun_tx.json").expect("read tx");
        let tx: EncodedConfirmedTransactionWithStatusMeta =
            serde_json::from_str(&sample_tx).expect("parse tx");
        let accounts = parse_pump_accounts(tx).expect("parse accounts");
        println!("{:?}", accounts);
        assert!(
            accounts.mint.to_string()
                == "6kPvKNrLqg23mApAvHzMKWohhVdSrA54HvrpYud8pump"
        );
        assert!(
            accounts.bonding_curve.to_string()
                == "6TGz5VAFF6UpSmTSk9327utugSWJCyVeVVFXDtZnMtNp"
        );
        assert!(
            accounts.associated_bonding_curve.to_string()
                == "4VwNGUif2ubbPjx4YNHmxEH7L4Yt2QFeo8uVTrVC3F68"
        );
        assert!(
            accounts.dev.to_string()
                == "2wgo94ZaiUNUkFBSKNaKsUgEANgSdex7gRpFKR39DPzw"
        );
    }

    #[tokio::test]
    async fn test_buy_pump_token() {
        dotenv::from_filename(".env").unwrap();
        // 0.00069 sol
        let lamports = 690000;
        let pump_accounts = PumpAccounts {
            mint: Pubkey::from_str(
                "5KEDcNGebCcLptWzknqVmPRNLHfiHA9Mm2djVE26pump",
            )
            .expect("parse mint"),
            bonding_curve: Pubkey::from_str(
                "Drhj4djqLsPyiA9qK2YmBngteFba8XhhvuQoBToW6pMS",
            )
            .expect("parse bonding curve"),
            associated_bonding_curve: Pubkey::from_str(
                "7uXq8diH862Dh8NgMHt5Tzsai8SvURhH58rArgxvs7o1",
            )
            .expect("parse associated bonding curve"),
            dev: Pubkey::from_str(
                "Gizxxed4uXCzL7Q8DyALDVoEEDfMkSV7XyUNrPDnPJ9J",
            )
            .expect("parse associated user"),
            metadata: Pubkey::default(), // not required
        };
        let wallet = Keypair::read_from_file(env("FUND_KEYPAIR_PATH"))
            .expect("read wallet");
        let rpc_client =
            RpcClient::new("https://api.mainnet-beta.solana.com".to_string());
        let auth = Arc::new(
            Keypair::read_from_file(env("AUTH_KEYPAIR_PATH")).unwrap(),
        );
        let mut searcher_client = Arc::new(Mutex::new(
            get_searcher_client(env("BLOCK_ENGINE_URL").as_str(), &auth)
                .await
                .expect("makes searcher client"),
        ));
        buy_pump_token(
            &wallet,
            &rpc_client,
            pump_accounts,
            lamports,
            &mut searcher_client,
            true,
        )
        .await
        .expect("buy pump token");
    }

    #[tokio::test]
    async fn test_get_bonding_curve_incomplete() {
        let rpc_client =
            RpcClient::new("https://api.mainnet-beta.solana.com".to_string());
        let bonding_curve_pubkey = Pubkey::from_str(
            "Drhj4djqLsPyiA9qK2YmBngteFba8XhhvuQoBToW6pMS", // some shitter
        )
        .expect("parse bonding curve");

        let bonding_curve =
            get_bonding_curve(&rpc_client, bonding_curve_pubkey)
                .await
                .expect("get bonding curve");

        println!("{:?}", bonding_curve);

        assert!(!bonding_curve.complete);
        assert_ne!(bonding_curve.virtual_token_reserves, 0);
        assert_ne!(bonding_curve.virtual_sol_reserves, 0);
        assert_ne!(bonding_curve.real_token_reserves, 0);
    }

    #[tokio::test]
    async fn test_get_bonding_curve_complete() {
        let rpc_client =
            RpcClient::new("https://api.mainnet-beta.solana.com".to_string());
        let bonding_curve_pubkey = Pubkey::from_str(
            "EB5tQ64HwNjaEoKKYAPkZqndwbULX249EuWSnkjfvR3y", // michi
        )
        .expect("parse bonding curve");

        let bonding_curve =
            get_bonding_curve(&rpc_client, bonding_curve_pubkey)
                .await
                .expect("get bonding curve");

        println!("{:?}", bonding_curve);

        assert!(bonding_curve.complete);
        assert_eq!(bonding_curve.virtual_token_reserves, 0);
        assert_eq!(bonding_curve.virtual_sol_reserves, 0);
        assert_eq!(bonding_curve.real_token_reserves, 0);
    }

    #[tokio::test]
    async fn test_get_token_amount() {
        // captured from prod
        let bonding_curve = BondingCurveLayout {
            blob1: 6966180631402821399,
            virtual_token_reserves: 1072964268463317,
            virtual_sol_reserves: 30000999057,
            real_token_reserves: 793064268463317,
            real_sol_reserves: 999057,
            blob4: 1000000000000000,
            complete: false,
        };
        let lamports = 500000;
        let expected_token_amount = 17852389307u64;
        let token_amount = get_token_amount(
            bonding_curve.virtual_sol_reserves,
            bonding_curve.virtual_token_reserves,
            Some(bonding_curve.real_token_reserves),
            lamports,
        )
        .expect("get token amount");
        // allow 10% less or more
        // 0.9 * expected < actual < 1.1 * expected
        let low_thresh = 0.9 * expected_token_amount as f64;
        let high_thresh = 1.1 * expected_token_amount as f64;
        let token_amount = token_amount as f64;
        assert!(token_amount >= low_thresh);
        assert!(token_amount <= high_thresh);
    }
}
