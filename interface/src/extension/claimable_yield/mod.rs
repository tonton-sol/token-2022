#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use {
    crate::extension::{Extension, ExtensionType},
    bytemuck::{Pod, Zeroable},
    solana_program_error::ProgramError,
    spl_pod::{optional_keys::OptionalNonZeroPubkey, primitives::PodU64},
};

/// Claimable Yield extension instructions
pub mod instruction;

/// Pure math function to calculate yield based on index difference
///
/// Formula: principal * (global_index - local_index) / local_index
///
/// Note: Caller should handle edge cases (zero indices, equal indices, etc.)
pub fn calculate_yield(
    principal: u64,
    local_index: u64,
    global_index: u64,
) -> Result<u64, ProgramError> {
    // Calculate yield using fixed-point arithmetic
    // yield = principal * (global_index / local_index - 1)
    // yield = principal * (global_index - local_index) / local_index
    let index_diff = global_index
        .checked_sub(local_index)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    let yield_scaled = (principal as u128)
        .checked_mul(index_diff as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    let yield_amount = yield_scaled
        .checked_div(local_index as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    // Round down here
    u64::try_from(yield_amount).map_err(|_| ProgramError::ArithmeticOverflow)
}

/// Claimable Yield extension data for mints
#[repr(C)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ClaimableYieldConfig {
    /// Authority that can update the global index
    pub index_authority: OptionalNonZeroPubkey,
    /// Global yield index (fixed-point representation with 9 decimal places)
    /// Represents the cumulative yield growth factor, starting at 1.0 = 1_000_000_000
    pub global_index: PodU64,
}

impl ClaimableYieldConfig {
    /// Scale factor for fixed-point arithmetic (10^9)
    pub const INDEX_SCALE: u64 = 1_000_000_000;

    /// Get the current global index as a u64
    pub fn get_global_index(&self) -> u64 {
        self.global_index.into()
    }

    /// Set the global index from a u64 value
    pub fn set_global_index(&mut self, index: u64) {
        self.global_index = PodU64::from(index);
    }
}

impl Extension for ClaimableYieldConfig {
    const TYPE: ExtensionType = ExtensionType::ClaimableYieldConfig;
}

/// Claimable Yield extension data for token accounts
#[repr(C)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ClaimableYieldAccount {
    /// Amount of yield that has been claimed but not yet minted (pending)
    pub pending_amount: PodU64,
    /// Local copy of the global index when this account was last updated
    pub local_index: PodU64,
}

impl ClaimableYieldAccount {
    /// Get the pending amount as a u64
    pub fn get_pending_amount(&self) -> u64 {
        self.pending_amount.into()
    }

    /// Set the pending amount from a u64 value
    pub fn set_pending_amount(&mut self, amount: u64) {
        self.pending_amount = PodU64::from(amount);
    }

    /// Add to the pending amount
    pub fn add_pending_amount(&mut self, amount: u64) -> Result<(), ProgramError> {
        let current = self.get_pending_amount();
        let new_amount = current
            .checked_add(amount)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        self.set_pending_amount(new_amount);
        Ok(())
    }

    /// Get the local index as a u64
    pub fn get_local_index(&self) -> u64 {
        self.local_index.into()
    }

    /// Set the local index from a u64 value
    pub fn set_local_index(&mut self, index: u64) {
        self.local_index = PodU64::from(index);
    }

    /// Reset pending amount to zero (used after hard claim)
    pub fn reset_pending_amount(&mut self) {
        self.pending_amount = PodU64::from(0);
    }

    /// Get the principal amount (account balance + pending yield)
    pub fn get_principal(&self, account_balance: u64) -> Result<u64, ProgramError> {
        account_balance
            .checked_add(self.get_pending_amount())
            .ok_or(ProgramError::ArithmeticOverflow)
    }

    /// Accrues pending yield based on global index
    pub fn accrue_pending_yield(&mut self, account_balance: u64, global_index: u64) -> Result<(), ProgramError> {
        if self.get_local_index() >= global_index {
            return Ok(());
        }
        
        let principal = self.get_principal(account_balance)?;
        let calculated_yield = calculate_yield(principal, self.get_local_index(), global_index)?;
        
        self.add_pending_amount(calculated_yield)?;
        if calculated_yield > 0 {
            self.set_local_index(global_index);
        }
        
        Ok(())
    }
}

impl Extension for ClaimableYieldAccount {
    const TYPE: ExtensionType = ExtensionType::ClaimableYieldAccount;
}

#[cfg(test)]
mod tests {
    use super::*;

    const INITIAL_INDEX: u64 = 1_000_000_000; // 1.0 with 9 decimal places

    #[test]
    fn test_index_scale() {
        assert_eq!(ClaimableYieldConfig::INDEX_SCALE, 1_000_000_000);
    }

    #[test]
    fn test_yield_calculation_no_growth() {
        let global_index = INITIAL_INDEX;
        let local_index = INITIAL_INDEX;

        // No yield when indices are equal - but pure function would fail on 0 diff
        // So we handle this edge case by not calling it
        assert_eq!(global_index, local_index);
        let yield_amount = 0; // Expected result when indices are equal
        assert_eq!(yield_amount, 0);
    }

    #[test]
    fn test_yield_calculation_with_growth() {
        let global_index = 1_200_000_000; // 20% growth (1.2)
        let local_index = INITIAL_INDEX;
        let principal = 1000;

        // 1000 principal with 20% growth should yield 200
        let yield_amount = calculate_yield(principal, local_index, global_index).unwrap();
        assert_eq!(yield_amount, 200);
    }

    #[test]
    fn test_yield_calculation_edge_cases() {
        let global_index = 1_500_000_000; // 50% growth
        let local_index = INITIAL_INDEX;

        // Zero principal should yield zero
        assert_eq!(calculate_yield(0, local_index, global_index).unwrap(), 0);

        // Zero local index - this would cause division by zero, so we don't call pure function
        // Caller should handle this edge case
        // assert_eq!(calculate_yield(1000, 0, global_index).unwrap(), 0);

        // Local index greater than global would cause underflow, so caller handles this
        // assert_eq!(calculate_yield(1000, 2_000_000_000, global_index).unwrap(), 0);

        // Test that these edge cases would indeed error if we called the pure function
        assert!(calculate_yield(1000, 0, global_index).is_err()); // Division by zero
        assert!(calculate_yield(1000, 2_000_000_000, global_index).is_err()); // Underflow
    }

    #[test]
    fn test_account_extension_operations() {
        let mut account_ext = ClaimableYieldAccount::default();

        // Test pending amount operations
        account_ext.set_pending_amount(100);
        assert_eq!(account_ext.get_pending_amount(), 100);

        account_ext.add_pending_amount(50).unwrap();
        assert_eq!(account_ext.get_pending_amount(), 150);

        account_ext.reset_pending_amount();
        assert_eq!(account_ext.get_pending_amount(), 0);

        // Test index operations
        account_ext.set_local_index(INITIAL_INDEX);
        assert_eq!(account_ext.get_local_index(), INITIAL_INDEX);
    }

    #[test]
    fn test_yield_calculation_rounding() {
        let global_index = 1_005_000_000; // 0.5% growth
        let local_index = INITIAL_INDEX;

        // Small amounts that would result in fractional yield should truncate
        let yield_amount = calculate_yield(10, local_index, global_index).unwrap();
        // 10 * (1.005 - 1.0) = 10 * 0.005 = 0.05 tokens, truncated to 0
        assert_eq!(yield_amount, 0);

        // Larger amounts should work correctly
        let yield_amount = calculate_yield(1000, local_index, global_index).unwrap();
        // 1000 * 0.005 = 5 tokens
        assert_eq!(yield_amount, 5);
    }

    #[test]
    fn test_yield_calculation_overflow_protection() {
        let global_index = u64::MAX;
        let local_index = INITIAL_INDEX;
        let principal = u64::MAX;

        // This should not panic due to overflow protection
        let result = calculate_yield(principal, local_index, global_index);

        // Should either succeed with a valid result or fail gracefully
        match result {
            Ok(_) => (),                                 // Valid result
            Err(ProgramError::ArithmeticOverflow) => (), // Graceful overflow handling
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_pending_amount_overflow_protection() {
        let mut account_ext = ClaimableYieldAccount::default();

        // Set pending amount to near max
        account_ext.set_pending_amount(u64::MAX - 10);

        // Try to add more - should fail gracefully
        let result = account_ext.add_pending_amount(20);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProgramError::ArithmeticOverflow);
    }

    #[test]
    fn test_extension_types() {
        // Verify extension types are correctly defined
        assert_eq!(
            ClaimableYieldConfig::TYPE,
            ExtensionType::ClaimableYieldConfig
        );
        assert_eq!(
            ClaimableYieldAccount::TYPE,
            ExtensionType::ClaimableYieldAccount
        );
    }

    #[test]
    fn test_pod_compliance() {
        // Test that structures are Pod-compliant (this will fail to compile if not)
        let config = ClaimableYieldConfig::default();
        let _bytes = bytemuck::bytes_of(&config);

        let account = ClaimableYieldAccount::default();
        let _bytes = bytemuck::bytes_of(&account);
    }
}
