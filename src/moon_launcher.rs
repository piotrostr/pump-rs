use solana_sdk::instruction::AccountMeta;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

use crate::constants::{
    ASSOCIATED_TOKEN_PROGRAM, SYSTEM_PROGRAM_ID, TOKEN_PROGRAM,
};
use crate::launcher::{
    derive_metadata_account, generate_mint, MPL_TOKEN_METADATA,
};
use crate::moonshot::{CONFIG_ACCOUNT, MOONSHOT_PROGRAM};

pub const BACKEND_AUTHORITY: &str =
    "Cb8Fnhp95f9dLxB3sYkNCbN3Mjxuc3v2uQZ7uVeqvNGB";

pub struct MoonshotLauncher {
    sender: Pubkey,
}

impl MoonshotLauncher {
    pub fn new(sender: Pubkey) -> Self {
        MoonshotLauncher { sender }
    }

    pub fn launch(&self) {
        let (mint, _mint_signer) = generate_mint();
        let _accounts = self._make_accounts(mint);
    }

    fn _make_accounts(&self, mint: Pubkey) -> Vec<AccountMeta> {
        let curve = find_curve_account(&mint);
        vec![
            AccountMeta::new(self.sender, true),
            AccountMeta::new_readonly(
                Pubkey::from_str(BACKEND_AUTHORITY).unwrap(),
                true,
            ),
            AccountMeta::new(curve, false),
            AccountMeta::new(mint, true),
            AccountMeta::new(derive_metadata_account(&mint), false),
            AccountMeta::new(
                spl_associated_token_account::get_associated_token_address(
                    &curve, &mint,
                ),
                false,
            ),
            AccountMeta::new_readonly(
                Pubkey::from_str(CONFIG_ACCOUNT).unwrap(),
                false,
            ),
            AccountMeta::new_readonly(
                Pubkey::from_str(TOKEN_PROGRAM).unwrap(),
                false,
            ),
            AccountMeta::new_readonly(
                Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM).unwrap(),
                false,
            ),
            AccountMeta::new_readonly(
                Pubkey::from_str(MPL_TOKEN_METADATA).unwrap(),
                false,
            ),
            AccountMeta::new_readonly(
                Pubkey::from_str(SYSTEM_PROGRAM_ID).unwrap(),
                false,
            ),
        ]
    }
}

pub fn find_curve_account(mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[&mint.to_bytes(), b"curve"],
        &Pubkey::from_str(MOONSHOT_PROGRAM).unwrap(),
    )
    .0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_curve_account() {
        let mint =
            Pubkey::from_str("9Ax6qJm5NhQQKgjf34P5rY4a749E1Y716VBxxYJZzTug")
                .unwrap();
        let curve = find_curve_account(&mint);
        assert_ne!(
            curve,
            Pubkey::from_str("BqGsLLLwU1ZiffKHf2bz7RNpqpyz39fuAHiyyhYqzuK1")
                .unwrap()
        );
    }
}
