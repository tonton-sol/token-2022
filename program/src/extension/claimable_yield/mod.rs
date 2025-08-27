/// Claimable Yield extension instructions
pub mod instruction;
/// Claimable Yield extension processor
pub mod processor;

pub use spl_token_2022_interface::extension::claimable_yield::{
    ClaimableYieldAccount, ClaimableYieldConfig,
};

use {
    crate::{
        error::TokenError,
        extension::{
            BaseStateWithExtensions, BaseStateWithExtensionsMut, PodStateWithExtensionsMut,
        },
        pod::{PodAccount, PodMint},
    },
    solana_program_error::ProgramError,
};

/// Updates claimable yield for an account (soft claim only)
pub fn update_account_yield<T: BaseStateWithExtensions<PodMint>>(
    account: &mut PodStateWithExtensionsMut<PodAccount>,
    mint: &T,
) -> Result<(), ProgramError> {
    if let Ok(mint_extension) = mint.get_extension::<ClaimableYieldConfig>() {
        if account.get_extension::<ClaimableYieldAccount>().is_ok() {
            let global_index = mint_extension.get_global_index();
            let account_amount = u64::from(account.base.amount);
            let (local_index, pending_amount) = {
                let yield_ext = account.get_extension::<ClaimableYieldAccount>()?;
                (yield_ext.get_local_index(), yield_ext.get_pending_amount())
            };

            let principal = account_amount
                .checked_add(pending_amount)
                .ok_or(TokenError::Overflow)?;
            let calculated_yield = mint_extension.calculate_yield(principal, local_index)?;

            let yield_ext = account.get_extension_mut::<ClaimableYieldAccount>()?;
            if calculated_yield > 0 {
                yield_ext.add_pending_amount(calculated_yield)?;
            }
            yield_ext.set_local_index(global_index);
        }
    }
    Ok(())
}
