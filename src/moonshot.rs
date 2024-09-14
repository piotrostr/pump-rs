use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

use crate::constants::SYSTEM_PROGRAM_ID;
use crate::util::make_compute_budget_ixs;

pub const DEX_FEE: &str = "3udvfL24waJcLhskRAsStNMoNUvtyXdxrWQz4hgi953N";
pub const HELIO_FEE: &str = "5K5RtTWzzLp4P8Npi84ocf7F1vBsAu29N1irG4iiUnzt";
pub const MOONSHOT_PROGRAM: &str =
    "MoonCVVNZFSYkqNXP6bxHLPL6QQJiMagDL3qcqUQTrG";
pub const CONFIG_ACCOUNT: &str =
    "36Eru7v11oU5Pfrojyn5oY3nETA1a1iqsw2WUu6afkM9";

#[derive(borsh::BorshSerialize)]
struct TradeParams {
    token_amount: u64,
    collateral_amount: u64, // lamports
    fixed_side: u8,
    slippage_bps: u64,
}

pub struct Moonshot {
    sender: Pubkey,
    curve_account: Pubkey,
    mint: Pubkey,
}

impl Moonshot {
    pub fn new(sender: Pubkey, curve_account: Pubkey, mint: Pubkey) -> Self {
        Self {
            sender,
            curve_account,
            mint,
        }
    }

    pub fn get_sender_token_account(&self) -> Pubkey {
        spl_associated_token_account::get_associated_token_address(
            &self.sender,
            &self.mint,
        )
    }

    pub fn get_curve_token_account(&self) -> Pubkey {
        spl_associated_token_account::get_associated_token_address(
            &self.curve_account,
            &self.mint,
        )
    }

    fn _make_swap_ix(
        &self,
        token_amount: u64,
        collateral_amount: u64,
        fixed_side: u8,
        slippage_bps: u64,
    ) -> Instruction {
        let trade_params = TradeParams {
            token_amount,
            collateral_amount,
            fixed_side,
            slippage_bps,
        };

        Instruction::new_with_borsh(
            Pubkey::from_str(MOONSHOT_PROGRAM).unwrap(),
            &trade_params,
            self._make_accounts(),
        )
    }

    fn _make_accounts(&self) -> Vec<AccountMeta> {
        vec![
            AccountMeta::new(self.sender, true),
            AccountMeta::new(self.get_sender_token_account(), false),
            AccountMeta::new(self.curve_account, false),
            AccountMeta::new(self.get_curve_token_account(), false),
            AccountMeta::new(Pubkey::from_str(DEX_FEE).unwrap(), false),
            AccountMeta::new(Pubkey::from_str(HELIO_FEE).unwrap(), false),
            AccountMeta::new(self.mint, false),
            AccountMeta::new(
                Pubkey::from_str(CONFIG_ACCOUNT).unwrap(),
                false,
            ),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(
                spl_associated_token_account::id(),
                false,
            ),
            AccountMeta::new_readonly(
                Pubkey::from_str(SYSTEM_PROGRAM_ID).unwrap(),
                false,
            ),
        ]
    }

    pub async fn buy(&self, token_amount: u64, collateral_amount: u64) {
        let mut ixs = vec![];
        ixs.append(&mut make_compute_budget_ixs(69, 200000));

        ixs.push(self._make_swap_ix(
            token_amount,
            collateral_amount,
            1,
            1000,
        ));
    }

    pub async fn sell(&self, token_amount: u64, collateral_amount: u64) {
        let mut ixs = vec![];
        ixs.append(&mut make_compute_budget_ixs(69, 200000));
        ixs.push(self._make_swap_ix(
            token_amount,
            collateral_amount,
            0,
            1000,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_moonshot() -> Moonshot {
        let sender =
            Pubkey::from_str("FASTykZyyjVfhutuRzMMYbFbFacQpRnMzDguhWfWadbi")
                .unwrap();
        let mint =
            Pubkey::from_str("CftroTurt6ZS7xsuvtL7bFynv3FZC7SswnpigmowNoJi")
                .unwrap();
        let curve_account =
            Pubkey::from_str("EEDcKnbAsCynaNUha5so9torbsaQqp5PHsgCMbXi5SN3")
                .unwrap();

        Moonshot::new(sender, curve_account, mint)
    }

    #[test]
    fn get_sender_token_account() {
        let moonshot = make_test_moonshot();
        let ata = moonshot.get_sender_token_account();
        assert_eq!(
            ata,
            Pubkey::from_str("BXkSyNk7jvTgt5fPgWELr4RoaeyZKg4qDowJGs5UQSkj")
                .unwrap()
        );
    }

    #[test]
    fn get_curve_token_account() {
        let moonshot = make_test_moonshot();
        let ata = moonshot.get_curve_token_account();
        assert_eq!(
            ata,
            Pubkey::from_str("ET3gCDj2GYHPhcAV2GDXkVYqeZ6xTtNj84VqgTwjXx1H")
                .unwrap()
        );
    }
}
