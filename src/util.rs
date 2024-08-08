use serde::Deserialize;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use std::cell::Cell;
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
    static COUNTER: Cell<u64> = Cell::new(1);
}

use log::info;

#[inline(always)]
#[timed::timed(duration(printer = "info!"))]
pub fn ultra_fast_random_0_to_7() -> u8 {
    COUNTER.with(|counter| {
        let current = counter.get();
        let new_value =
            current.wrapping_mul(6364136223846793005).wrapping_add(1);
        counter.set(new_value);
        (new_value >> 61) as u8
    })
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
    let index = ultra_fast_random_0_to_7();
    Pubkey::from_str(PUBKEYS[index as usize]).expect("parse tip pubkey")
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_get_jito_tip_pubkey() {
        let start = std::time::Instant::now();
        let _ = super::get_jito_tip_pubkey();
        let elapsed = start.elapsed();
        println!("elapsed: {:?}", elapsed);
    }
}
