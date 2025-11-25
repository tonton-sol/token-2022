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

fn process_configure_account(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let token_account_info = next_account_info(account_info_iter)?;
    let mint_account_info = next_account_info(account_info_iter)?;
    let owner_info = next_account_info(account_info_iter)?;
    let owner_info_data_len = owner_info.data_len();

    // First, get the global index from the mint
    let mut mint_data = mint_account_info.data.borrow_mut();
    let mint = PodStateWithExtensionsMut::<PodMint>::unpack(&mut mint_data)?;
    let mint_extension = mint.get_extension::<ClaimableYieldConfig>()?;
    let global_index = mint_extension.get_global_index();
    drop(mint_data);

    // Now configure the token account
    let mut token_account_data = token_account_info.data.borrow_mut();
    let mut token_account =
        PodStateWithExtensionsMut::<PodAccount>::unpack(&mut token_account_data)?;

    // Validate owner
    Processor::validate_owner(
        program_id,
        &token_account.base.owner,
        owner_info,
        owner_info_data_len,
        account_info_iter.as_slice(),
    )?;

    // Check if extension already exists
    if token_account
        .get_extension::<ClaimableYieldAccount>()
        .is_ok()
    {
        return Err(TokenError::ExtensionAlreadyInitialized.into());
    }

    // Initialize the extension with current global index
    let extension = token_account.init_extension::<ClaimableYieldAccount>(false)?;
    extension.set_local_index(global_index);
    extension.set_pending_amount(0);

    Ok(())
}

fn process_claim_yield(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let token_account_info = next_account_info(account_info_iter)?;
    let mint_account_info = next_account_info(account_info_iter)?;
    let yield_authority_info = next_account_info(account_info_iter)?;
    let owner_info_data_len = yield_authority_info.data_len();

    // Get token account data
    let mut token_account_data = token_account_info.data.borrow_mut();
    let mut token_account =
        PodStateWithExtensionsMut::<PodAccount>::unpack(&mut token_account_data)?;
    let owner = token_account.base.owner;

    // If yield eligible, verify yield authority is owner
    // Else, verify provided yield authority matches mint yield authority
    // Validate owner
    // Processor::validate_owner(
    //     program_id,
    //     &token_account.base.owner,
    //     yield_authority_info,
    //     owner_info_data_len,
    //     account_info_iter.as_slice(),
    // )?;

    // Extract account amount before mutable borrow
    let account_amount = u64::from(token_account.base.amount);

    let mut mint_data = mint_account_info.data.borrow_mut();
    let mint = PodStateWithExtensionsMut::<PodMint>::unpack(&mut mint_data)?;

    // Accrue pending yield
    let account_extension = token_account.get_extension_mut::<ClaimableYieldAccount>()?;
    let mint_extension = mint.get_extension::<ClaimableYieldConfig>()?;

    let yield_authority = Option::<Pubkey>::from(mint_extension.yield_authority);

    // Validate the signer based on yield authority and eligibility
    match yield_authority {
        Some(auth) if !bool::from(account_extension.yield_eligible) => {
            // Yield authority is set and account is not marked as eligible, yield authority must be the one claiming yield
            if *yield_authority_info.key != auth {
                return Err(TokenError::AuthorityTypeNotSupported.into());
            }
        }
        _ => {
            // Either yield authority is not set or account is flagged as eligible for yield collection, verify token account owner is collecting yield
            Processor::validate_owner(
                program_id,
                &owner,
                yield_authority_info,
                owner_info_data_len,
                account_info_iter.as_slice(),
            )?;
        }
    }

    account_extension.accrue_pending_yield(account_amount, mint_extension.get_global_index())?;

    let total_yield = account_extension.get_pending_amount();
    if total_yield > 0 {
        // Reset pending amount
        account_extension.reset_pending_amount();

        // Update mint supply
        mint.base.supply = u64::from(mint.base.supply)
            .checked_add(total_yield)
            .ok_or(TokenError::Overflow)?
            .into();

        // Update token account balance
        token_account.base.amount = u64::from(token_account.base.amount)
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
        ClaimableYieldInstruction::UpdateIndex => {
            msg!("ClaimableYieldInstruction::UpdateIndex");
            let UpdateIndexInstructionData { new_index } = decode_instruction_data(input)?;
            process_update_index(program_id, accounts, u64::from(*new_index))
        }
        ClaimableYieldInstruction::ConfigureAccount => {
            msg!("ClaimableYieldInstruction::ConfigureAccount");
            process_configure_account(program_id, accounts)
        }
        ClaimableYieldInstruction::ClaimYield => {
            msg!("ClaimableYieldInstruction::ClaimYield");
            process_claim_yield(program_id, accounts)
        }
    }
}
