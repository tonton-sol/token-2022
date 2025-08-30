/// Claimable Yield extension instructions
pub mod instruction;
/// Claimable Yield extension processor
pub mod processor;

use crate::{
    extension::{BaseStateWithExtensions, BaseStateWithExtensionsMut, PodStateWithExtensionsMut},
    pod::{PodAccount, PodMint},
};
use solana_program_error::ProgramError;

pub use spl_token_2022_interface::extension::claimable_yield::{
    calculate_yield, ClaimableYieldAccount, ClaimableYieldConfig,
};

/// Checks for yield and applies it to the pending yield if needed
/// Gracefully handles accounts without ClaimableYieldAccount extension.
pub fn checked_accrue_pending_yield<T>(
    account: &mut PodStateWithExtensionsMut<PodAccount>,
    mint: &T,
) -> Result<(), ProgramError>
where
    T: BaseStateWithExtensions<PodMint>,
{
    // Check if account has claimable yield extension
    if account.get_extension::<ClaimableYieldAccount>().is_ok() {
        let global_index = mint
            .get_extension::<ClaimableYieldConfig>()?
            .get_global_index();

        let account_amount = u64::from(account.base.amount);
        let account_extension = account.get_extension_mut::<ClaimableYieldAccount>()?;

        // Accrue yield to the pending yield
        account_extension.accrue_pending_yield(account_amount, global_index)?;
    }
    Ok(())
}
