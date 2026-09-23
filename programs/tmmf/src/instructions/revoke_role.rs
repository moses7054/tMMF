use anchor_lang::prelude::*;

use crate::{error::ErrorCode, events::RoleRevoked, require_own_program_data, Role, RoleType};

#[derive(Accounts)]
#[instruction(role: RoleType, user: Pubkey)]
pub struct RevokeRole<'info> {
    #[account(mut)]
    pub rent_recipient: SystemAccount<'info>,

    pub authority: Signer<'info>,

    #[account(
        mut,
        close = rent_recipient,
        seeds = [role.seed(), user.as_ref()],
        bump = role_to_revoke.bump,
    )]
    pub role_to_revoke: Account<'info, Role>,

    #[account(address = crate::ID)]
    pub program: Program<'info, crate::program::Tmmf>,

    #[account(
        constraint =
            program_data.upgrade_authority_address == Some(authority.key()) @ ErrorCode::NotUpgradeAuthority
    )]
    pub program_data: Account<'info, ProgramData>,
}

impl<'info> RevokeRole<'info> {
    pub fn revoke_role(&mut self, role: RoleType, user: Pubkey) -> Result<()> {
        require_own_program_data(&self.program, &self.program_data)?;

        emit!(RoleRevoked {
            role,
            grantee: user,
            revoker: self.authority.key(),
        });
        Ok(())
    }
}
