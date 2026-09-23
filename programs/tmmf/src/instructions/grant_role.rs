use anchor_lang::prelude::*;

use crate::{error::ErrorCode, events::RoleGranted, require_own_program_data, Role, RoleType};

#[derive(Accounts)]
#[instruction(role: RoleType, user: Pubkey)]
pub struct GrantRole<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    pub authority: Signer<'info>,

    #[account(
        init,
        payer = payer,
        space = Role::DISCRIMINATOR.len() + Role::INIT_SPACE,
        seeds = [role.seed(), user.as_ref()],
        bump
    )]
    pub role_to_grant: Account<'info, Role>,

    #[account(address = crate::ID)]
    pub program: Program<'info, crate::program::Tmmf>,

    #[account(
        constraint =
            program_data.upgrade_authority_address == Some(authority.key()) @ ErrorCode::NotUpgradeAuthority
    )]
    pub program_data: Account<'info, ProgramData>,

    pub system_program: Program<'info, System>,
}

impl<'info> GrantRole<'info> {
    pub fn grant_role(&mut self, role: RoleType, user: Pubkey, bump: u8) -> Result<()> {
        require_own_program_data(&self.program, &self.program_data)?;

        self.role_to_grant.set_inner(Role {
            address: user,
            role,
            bump,
        });

        emit!(RoleGranted {
            role,
            grantee: user,
            granter: self.authority.key(),
        });
        Ok(())
    }
}
