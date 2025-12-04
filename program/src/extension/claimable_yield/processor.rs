use {
    crate::{
        check_program_account,
        error::TokenError,
        extension::{
            claimable_yield::{
                instruction::{
                    ClaimableYieldInstruction, InitializeInstructionData,
                    UpdateIndexInstructionData,
                },
                ClaimableYieldAccount, ClaimableYieldConfig,
            },
            BaseStateWithExtensions, BaseStateWithExtensionsMut, PodStateWithExtensionsMut,
        },
        instruction::{decode_instruction_data, decode_instruction_type},
        pod::{PodAccount, PodMint},
        processor::Processor,
    },
    solana_account_info::{next_account_info, AccountInfo},
    solana_msg::msg,
    solana_program_error::ProgramResult,
    solana_pubkey::Pubkey,
    spl_pod::optional_keys::OptionalNonZeroPubkey,
    spl_token_2022_interface::extension::PodStateWithExtensions,
};

fn process_initialize_mint(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    yield_authority: &OptionalNonZeroPubkey,
    index_authority: &OptionalNonZeroPubkey,
    initial_index: u64,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let mint_account_info = next_account_info(account_info_iter)?;

    let mut mint_data = mint_account_info.data.borrow_mut();
    let mut mint = PodStateWithExtensionsMut::<PodMint>::unpack_uninitialized(&mut mint_data)?;

    let extension = mint.init_extension::<ClaimableYieldConfig>(true)?;
    extension.yield_authority = *yield_authority;
    extension.index_authority = *index_authority;
    extension.set_global_index(initial_index);

    Ok(())
}

fn process_enable_yield(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let token_account_info = next_account_info(account_info_iter)?;
    let mint_account_info = next_account_info(account_info_iter)?;
    let yield_authority_account_info = next_account_info(account_info_iter)?;
    let yield_authority_info_data_len = yield_authority_account_info.data_len();

    let mut token_account_data = token_account_info.data.borrow_mut();
    let mut token_account =
        PodStateWithExtensionsMut::<PodAccount>::unpack(&mut token_account_data)?;
    if token_account.base.mint != *mint_account_info.key {
        return Err(TokenError::MintMismatch.into());
    }
    let token_account_extension = token_account.get_extension_mut::<ClaimableYieldAccount>()?;

    let mint_data = mint_account_info.data.borrow();
    let mint = PodStateWithExtensions::<PodMint>::unpack(&mint_data)?;
    let mint_extension = mint.get_extension::<ClaimableYieldConfig>()?;

    let authority = Option::<Pubkey>::from(mint_extension.yield_authority)
        .ok_or(TokenError::NoAuthorityExists)?;

    Processor::validate_owner(
        program_id,
        &authority,
        yield_authority_account_info,
        yield_authority_info_data_len,
        account_info_iter.as_slice(),
    )?;

    // If account is already flagged eligible, return an error
    if token_account_extension.get_yield_eligible() {
        return Err(TokenError::InvalidState.into());
    }

    token_account_extension.set_yield_eligible(true);

    Ok(())
}

fn process_disable_yield(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let token_account_info = next_account_info(account_info_iter)?;
    let mint_account_info = next_account_info(account_info_iter)?;
    let yield_authority_account_info = next_account_info(account_info_iter)?;
    let yield_authority_info_data_len = yield_authority_account_info.data_len();

    let mut token_account_data = token_account_info.data.borrow_mut();
    let mut token_account =
        PodStateWithExtensionsMut::<PodAccount>::unpack(&mut token_account_data)?;
    if token_account.base.mint != *mint_account_info.key {
        return Err(TokenError::MintMismatch.into());
    }
    let token_account_extension = token_account.get_extension_mut::<ClaimableYieldAccount>()?;

    let mint_data = mint_account_info.data.borrow();
    let mint = PodStateWithExtensions::<PodMint>::unpack(&mint_data)?;
    let mint_extension = mint.get_extension::<ClaimableYieldConfig>()?;

    let authority = Option::<Pubkey>::from(mint_extension.yield_authority)
        .ok_or(TokenError::NoAuthorityExists)?;

    Processor::validate_owner(
        program_id,
        &authority,
        yield_authority_account_info,
        yield_authority_info_data_len,
        account_info_iter.as_slice(),
    )?;

    // If account is already not eligible, return an error
    if !token_account_extension.get_yield_eligible() {
        return Err(TokenError::InvalidState.into());
    }

    token_account_extension.set_yield_eligible(false);

    Ok(())
}

fn process_update_index(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    new_index: u64,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let mint_account_info = next_account_info(account_info_iter)?;
    let authority_info = next_account_info(account_info_iter)?;
    let authority_info_data_len = authority_info.data_len();

    let mut mint_data = mint_account_info.data.borrow_mut();
    let mut mint = PodStateWithExtensionsMut::<PodMint>::unpack(&mut mint_data)?;
    let extension = mint.get_extension_mut::<ClaimableYieldConfig>()?;

    let authority =
        Option::<Pubkey>::from(extension.index_authority).ok_or(TokenError::NoAuthorityExists)?;

    Processor::validate_owner(
        program_id,
        &authority,
        authority_info,
        authority_info_data_len,
        account_info_iter.as_slice(),
    )?;

    // Ensure new index is not less than current index to prevent yield reduction
    let current_index = extension.get_global_index();
    if new_index < current_index {
        return Err(TokenError::InvalidInstruction.into());
    }

    extension.set_global_index(new_index);
    Ok(())
}

fn process_claim_yield_to(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let source_account_info = next_account_info(account_info_iter)?;
    let target_account_info = next_account_info(account_info_iter)?;
    let mint_account_info = next_account_info(account_info_iter)?;
    let authority_info = next_account_info(account_info_iter)?;
    let authority_info_data_len = authority_info.data_len();

    let mut source_account_data = source_account_info.data.borrow_mut();
    let mut source_account =
        PodStateWithExtensionsMut::<PodAccount>::unpack(&mut source_account_data)?;
    // Extract source amount and owner before mutable borrow
    let source_account_owner = source_account.base.owner;
    let source_account_amount = u64::from(source_account.base.amount);

    if source_account.base.mint != *mint_account_info.key {
        return Err(TokenError::MintMismatch.into());
    }

    let mut target_account_data = target_account_info.data.borrow_mut();
    let target_account = PodStateWithExtensionsMut::<PodAccount>::unpack(&mut target_account_data)?;

    if target_account.base.mint != *mint_account_info.key {
        return Err(TokenError::MintMismatch.into());
    }

    let mut mint_data = mint_account_info.data.borrow_mut();
    let mint = PodStateWithExtensionsMut::<PodMint>::unpack(&mut mint_data)?;
    let mint_extension = mint.get_extension::<ClaimableYieldConfig>()?;

    let source_extension = source_account.get_extension_mut::<ClaimableYieldAccount>()?;

    if source_extension.get_yield_eligible() {
        // Source account is yield eligible, the token account owner must be the signer
        Processor::validate_owner(
            program_id,
            &source_account_owner,
            authority_info,
            authority_info_data_len,
            account_info_iter.as_slice(),
        )?;
    } else {
        // Source account is not yield eligible, the mint yield authority must exist and be the signer
        let yield_authority = Option::<Pubkey>::from(mint_extension.yield_authority)
            .ok_or(TokenError::NoAuthorityExists)?;

        Processor::validate_owner(
            program_id,
            &yield_authority,
            authority_info,
            authority_info_data_len,
            account_info_iter.as_slice(),
        )?;
    }

    source_extension
        .accrue_pending_yield(source_account_amount, mint_extension.get_global_index())?;

    let total_yield = source_extension.get_pending_amount();
    if total_yield > 0 {
        source_extension.reset_pending_amount();

        mint.base.supply = u64::from(mint.base.supply)
            .checked_add(total_yield)
            .ok_or(TokenError::Overflow)?
            .into();

        target_account.base.amount = u64::from(target_account.base.amount)
            .checked_add(total_yield)
            .ok_or(TokenError::Overflow)?
            .into();
    }

    Ok(())
}

pub(crate) fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    input: &[u8],
) -> ProgramResult {
    check_program_account(program_id)?;
    match decode_instruction_type(input)? {
        ClaimableYieldInstruction::InitializeMint => {
            msg!("ClaimableYieldInstruction::InitializeMint");
            let InitializeInstructionData {
                yield_authority,
                index_authority,
                initial_index,
            } = decode_instruction_data(input)?;
            process_initialize_mint(
                program_id,
                accounts,
                yield_authority,
                index_authority,
                u64::from(*initial_index),
            )
        }
        ClaimableYieldInstruction::EnableYield => {
            msg!("ClaimableYieldInstruction::EnableYield");
            process_enable_yield(program_id, accounts)
        }
        ClaimableYieldInstruction::DisableYield => {
            msg!("ClaimableYieldInstruction::DisableYield");
            process_disable_yield(program_id, accounts)
        }
        ClaimableYieldInstruction::ClaimYieldTo => {
            msg!("ClaimableYieldInstruction::ClaimYieldTo");
            process_claim_yield_to(program_id, accounts)
        }
        ClaimableYieldInstruction::UpdateIndex => {
            msg!("ClaimableYieldInstruction::UpdateIndex");
            let UpdateIndexInstructionData { new_index } = decode_instruction_data(input)?;
            process_update_index(program_id, accounts, u64::from(*new_index))
        }
    }
}
