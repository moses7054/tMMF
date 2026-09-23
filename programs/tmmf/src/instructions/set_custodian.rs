use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::{Fund, Role, RoleType, FUND_SEED};

#[derive(Accounts)]
pub struct SetCustodian<'info> {
    pub admin: Signer<'info>,

    #[account(
        seeds = [RoleType::FUND_ADMIN_SEED, admin.key().as_ref()],
        bump = admin_role.bump,
    )]
    pub admin_role: Account<'info, Role>,

    #[account(mut, seeds = [FUND_SEED], bump = fund.bump, has_one = usdc_mint)]
    pub fund: Account<'info, Fund>,

    #[account(mint::token_program = usdc_token_program)]
    pub usdc_mint: InterfaceAccount<'info, Mint>,

    #[account(token::mint = usdc_mint, token::token_program = usdc_token_program)]
    pub custodian: InterfaceAccount<'info, TokenAccount>,

    pub usdc_token_program: Interface<'info, TokenInterface>,
}

impl<'info> SetCustodian<'info> {
    pub fn set_custodian(&mut self) -> Result<()> {
        self.fund.custodian = self.custodian.key();
        Ok(())
    }
}
