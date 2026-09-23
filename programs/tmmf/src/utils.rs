use anchor_lang::prelude::*;

use crate::error::ErrorCode;

pub fn require_own_program_data(
    program: &Program<crate::program::Tmmf>,
    program_data: &Account<ProgramData>,
) -> Result<()> {
    let expected = program
        .programdata_address()?
        .ok_or(ErrorCode::NotUpgradeAuthority)?;
    require_keys_eq!(expected, program_data.key(), ErrorCode::NotUpgradeAuthority);
    Ok(())
}
