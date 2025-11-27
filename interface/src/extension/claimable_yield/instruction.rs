#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use {
    crate::{
        check_program_account,
        instruction::{encode_instruction, TokenInstruction},
    },
    bytemuck::{Pod, Zeroable},
    num_enum::{IntoPrimitive, TryFromPrimitive},
    solana_instruction::{AccountMeta, Instruction},
    solana_program_error::ProgramError,
    solana_pubkey::Pubkey,
    spl_pod::{optional_keys::OptionalNonZeroPubkey, primitives::PodU64},
    std::convert::TryInto,
};

/// Claimable Yield extension instructions
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[derive(Clone, Copy, Debug, PartialEq, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
pub enum ClaimableYieldInstruction {
    /// Initialize a mint with claimable yield functionality.
    ///
    /// Fails if the mint has already been initialized, so must be called before
    /// `InitializeMint`.
    ///
    /// The mint must have exactly enough space allocated for the base mint (82
    /// bytes), plus 83 bytes of padding, 1 byte reserved for the account type,
    /// then space required for this extension, plus any others.
    ///
    /// Accounts expected by this instruction:
    ///
    ///   0. `[writable]` The mint to initialize.
    ///
    /// Data expected by this instruction:
    ///   `crate::extension::claimable_yield::instruction::InitializeInstructionData`
    InitializeMint,

    /// Update the global yield index. Only supported for mints that include the
    /// `ClaimableYield` extension.
    ///
    /// The authority provides a new index value which represents the cumulative
    /// growth factor. The index must be greater than or equal to the current
    /// global index to prevent yield reduction.
    ///
    /// Accounts expected by this instruction:
    ///
    ///   * Single authority
    ///   0. `[writable]` The mint.
    ///   1. `[signer]` The index authority.
    ///
    ///   * Multisignature authority
    ///   0. `[writable]` The mint.
    ///   1. `[]` The mint's multisignature index authority.
    ///   2. `..2+M` `[signer]` M signer accounts.
    ///
    /// Data expected by this instruction:
    ///   `crate::extension::claimable_yield::instruction::UpdateIndexInstructionData`
    UpdateIndex,

    /// Claim all accrued yield for a token account.
    ///
    /// This instruction performs both soft and hard claims in a single operation:
    /// 1. Calculates all unclaimed yield based on the current global index
    /// 2. Adds any existing pending amount to the calculated yield
    /// 3. Mints new tokens equal to the total yield amount
    /// 4. Adds the minted tokens to the account's spendable balance
    /// 5. Updates the account's local index to the current global index
    /// 6. Resets the pending amount to zero
    ///
    /// The account extension must already be configured for claimable yield.
    ///
    /// Accounts expected by this instruction:
    ///
    ///   0. `[writable]` The token account to claim yield for.
    ///   1. `[writable]` The mint account (to increase supply).
    ///   2. `[signer]` The account's yield authority.
    ///
    ///   * Multisignature authority
    ///   0. `[writable]` The token account to claim yield for.
    ///   1. `[writable]` The mint account.
    ///   2. `[]` The account's multisignature owner.
    ///   3. `..3+M` `[signer]` M signer accounts.
    ClaimYield,
}

/// Data expected by `ClaimableYieldInstruction::InitializeMint`
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct InitializeInstructionData {
    /// The public key for the account which has authority over which accounts can claim yield. If None, token account owners may always claim yield.
    pub yield_authority: OptionalNonZeroPubkey,
    /// The public key for the account that can update the global index
    pub index_authority: OptionalNonZeroPubkey,
    /// The initial global index value (fixed-point with 9 decimal places)
    pub initial_index: PodU64,
}

/// Data expected by `ClaimableYieldInstruction::UpdateIndex`
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct UpdateIndexInstructionData {
    /// The new global index value (fixed-point with 9 decimal places)
    pub new_index: PodU64,
}

/// Create an `InitializeMint` instruction
pub fn initialize_mint(
    token_program_id: &Pubkey,
    mint: &Pubkey,
    yield_authority: Option<Pubkey>,
    index_authority: Option<Pubkey>,
    initial_index: u64,
) -> Result<Instruction, ProgramError> {
    check_program_account(token_program_id)?;
    let accounts = vec![AccountMeta::new(*mint, false)];
    Ok(encode_instruction(
        token_program_id,
        accounts,
        TokenInstruction::ClaimableYieldExtension,
        ClaimableYieldInstruction::InitializeMint,
        &InitializeInstructionData {
            yield_authority: yield_authority.try_into()?,
            index_authority: index_authority.try_into()?,
            initial_index: initial_index.into(),
        },
    ))
}

/// Create an `UpdateIndex` instruction
pub fn update_index(
    token_program_id: &Pubkey,
    mint: &Pubkey,
    index_authority: &Pubkey,
    signers: &[&Pubkey],
    new_index: u64,
) -> Result<Instruction, ProgramError> {
    check_program_account(token_program_id)?;
    let mut accounts = vec![
        AccountMeta::new(*mint, false),
        AccountMeta::new_readonly(*index_authority, signers.is_empty()),
    ];
    for signer_pubkey in signers.iter() {
        accounts.push(AccountMeta::new_readonly(**signer_pubkey, true));
    }
    Ok(encode_instruction(
        token_program_id,
        accounts,
        TokenInstruction::ClaimableYieldExtension,
        ClaimableYieldInstruction::UpdateIndex,
        &UpdateIndexInstructionData {
            new_index: new_index.into(),
        },
    ))
}

/// Create a `ClaimYield` instruction
pub fn claim_yield(
    token_program_id: &Pubkey,
    account: &Pubkey,
    mint: &Pubkey,
    owner: &Pubkey,
    signers: &[&Pubkey],
) -> Result<Instruction, ProgramError> {
    check_program_account(token_program_id)?;
    let mut accounts = vec![
        AccountMeta::new(*account, false),
        AccountMeta::new(*mint, false), // Mint needs to be writable for supply update
        AccountMeta::new_readonly(*owner, signers.is_empty()),
    ];
    for signer_pubkey in signers.iter() {
        accounts.push(AccountMeta::new_readonly(**signer_pubkey, true));
    }
    Ok(encode_instruction(
        token_program_id,
        accounts,
        TokenInstruction::ClaimableYieldExtension,
        ClaimableYieldInstruction::ClaimYield,
        &(),
    ))
}
