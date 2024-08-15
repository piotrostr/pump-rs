use chrono::Local;
use env_logger::Builder;
use log::LevelFilter;
use rand::rngs::ThreadRng;
use rand::thread_rng;
use rand::Rng;
use serde::Deserialize;
use solana_account_decoder::parse_account_data::ParsedAccount;
use solana_account_decoder::UiAccountData;
use solana_client::rpc_response::RpcKeyedAccount;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::EncodableKey;
use std::cell::RefCell;
use std::error::Error;
use std::io::Write;
use std::str::FromStr;

pub fn env(var: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| panic!("{} env var not set", var))
}

/// Helper function for pubkey serialize
pub fn pubkey_to_string<S>(
    pubkey: &Pubkey,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&pubkey.to_string())
}

/// Helper function for pubkey deserialize
pub fn string_to_pubkey<'de, D>(deserializer: D) -> Result<Pubkey, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Pubkey::from_str(&s).map_err(serde::de::Error::custom)
}

pub fn string_to_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    s.parse().map_err(serde::de::Error::custom)
}

pub fn make_compute_budget_ixs(
    price: u64,
    max_units: u32,
) -> Vec<Instruction> {
    vec![
        ComputeBudgetInstruction::set_compute_unit_price(price),
        ComputeBudgetInstruction::set_compute_unit_limit(max_units),
    ]
}

thread_local! {
    static RNG: RefCell<ThreadRng> = RefCell::new(thread_rng());
}

#[inline(always)]
pub fn fast_random_0_to_7() -> u8 {
    RNG.with(|rng| rng.borrow_mut().gen_range(0..8))
}

pub fn get_jito_tip_pubkey() -> Pubkey {
    const PUBKEYS: [&str; 8] = [
        "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5",
        "HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe",
        "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY",
        "ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49",
        "DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh",
        "ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt",
        "DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL",
        "3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT",
    ];
    let index = fast_random_0_to_7();
    Pubkey::from_str(PUBKEYS[index as usize]).expect("parse tip pubkey")
}

#[cfg(test)]
mod tests {
    #[test]
    fn bench_get_jito_tip_pubkey() {
        for _ in 0..100 {
            let start = std::time::Instant::now();
            let _ = super::get_jito_tip_pubkey();
            let elapsed = start.elapsed();
            println!("elapsed: {:?}", elapsed);
        }
    }
}
#[derive(Debug, Default, Clone)]
pub struct Holding {
    pub mint: Pubkey,
    pub ata: Pubkey,
    pub amount: u64,
}

pub fn parse_holding(
    ata: RpcKeyedAccount,
) -> Result<Holding, Box<dyn Error>> {
    if let UiAccountData::Json(ParsedAccount {
        program: _,
        parsed,
        space: _,
    }) = ata.account.data
    {
        let amount = parsed["info"]["tokenAmount"]["amount"]
            .as_str()
            .expect("amount")
            .parse::<u64>()?;
        let mint =
            Pubkey::from_str(parsed["info"]["mint"].as_str().expect("mint"))?;
        let ata = Pubkey::from_str(&ata.pubkey)?;
        Ok(Holding { mint, ata, amount })
    } else {
        Err("failed to parse holding".into())
    }
}

pub fn read_fund_keypair() -> Keypair {
    Keypair::read_from_file(env("FUND_KEYPAIR_PATH"))
        .expect("read fund keypair")
}

pub fn init_logger() -> Result<(), Box<dyn Error>> {
    let logs_level = match std::env::var("RUST_LOG") {
        Ok(level) => {
            LevelFilter::from_str(&level).unwrap_or(LevelFilter::Info)
        }
        Err(_) => LevelFilter::Info,
    };

    // in logs, use unix timestamp in ms
    Builder::from_default_env()
        .format(|buf, record| {
            writeln!(
                buf,
                "{} [{}] {}",
                Local::now().timestamp_millis(),
                record.level(),
                record.args()
            )
        })
        .filter(None, logs_level)
        .try_init()?;

    Ok(())
}
