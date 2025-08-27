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

    /// Calculate unclaimed yield for a given principal and local index
    ///
    /// Formula: principal * (global_index / local_index - 1)
    ///
    /// Returns the yield amount, or an error if calculation would overflow
    pub fn calculate_yield(&self, principal: u64, local_index: u64) -> Result<u64, ProgramError> {
        if local_index == 0 || local_index > u64::from(self.global_index) {
            return Ok(0);
        }

        let global_index: u64 = self.global_index.into();
        if global_index <= local_index {
            return Ok(0);
        }

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
        let config = ClaimableYieldConfig {
            global_index: PodU64::from(INITIAL_INDEX),
            ..Default::default()
        };

        // No yield when indices are equal
        let yield_amount = config.calculate_yield(1000, INITIAL_INDEX).unwrap();
        assert_eq!(yield_amount, 0);
    }

    #[test]
    fn test_yield_calculation_with_growth() {
        let config = ClaimableYieldConfig {
            global_index: PodU64::from(1_200_000_000), // 20% growth (1.2)
            ..Default::default()
        };

        // 1000 principal with 20% growth should yield 200
        let yield_amount = config.calculate_yield(1000, INITIAL_INDEX).unwrap();
        assert_eq!(yield_amount, 200);
    }

    #[test]
    fn test_yield_calculation_edge_cases() {
        let config = ClaimableYieldConfig {
            global_index: PodU64::from(1_500_000_000), // 50% growth
            ..Default::default()
        };

        // Zero principal should yield zero
        assert_eq!(config.calculate_yield(0, INITIAL_INDEX).unwrap(), 0);

        // Zero local index should yield zero (safety check)
        assert_eq!(config.calculate_yield(1000, 0).unwrap(), 0);

        // Local index greater than global should yield zero
        assert_eq!(config.calculate_yield(1000, 2_000_000_000).unwrap(), 0);
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
        let config = ClaimableYieldConfig {
            global_index: PodU64::from(1_005_000_000), // 0.5% growth
            ..Default::default()
        };

        // Small amounts that would result in fractional yield should truncate
        let yield_amount = config.calculate_yield(10, INITIAL_INDEX).unwrap();
        // 10 * (1.005 - 1.0) = 10 * 0.005 = 0.05 tokens, truncated to 0
        assert_eq!(yield_amount, 0);

        // Larger amounts should work correctly
        let yield_amount = config.calculate_yield(1000, INITIAL_INDEX).unwrap();
        // 1000 * 0.005 = 5 tokens
        assert_eq!(yield_amount, 5);
    }

    #[test]
    fn test_yield_calculation_overflow_protection() {
        let config = ClaimableYieldConfig {
            global_index: PodU64::from(u64::MAX),
            ..Default::default()
        };

        // This should not panic due to overflow protection
        let result = config.calculate_yield(u64::MAX, INITIAL_INDEX);

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
