use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    transfer_checked, Mint, Token2022, TokenAccount, TokenInterface, TransferChecked,
};

use crate::{error::ErrorCode, Fund, Role, RoleType, FUND_SEED};

#[derive(Accounts)]
pub struct SweepToCustodian<'info> {
    pub admin: Signer<'info>,

    #[account(
        seeds = [RoleType::FUND_ADMIN_SEED, admin.key().as_ref()],
        bump = admin_role.bump,
    )]
    pub admin_role: Account<'info, Role>,

    #[account(
        seeds = [FUND_SEED],
        bump = fund.bump,
        has_one = usdc_mint,
        has_one = custodian,
        has_one = vault,
        has_one = share_mint,
    )]
    pub fund: Account<'info, Fund>,

    #[account(mint::token_program = usdc_token_program)]
    pub usdc_mint: InterfaceAccount<'info, Mint>,

    #[account(mint::token_program = share_token_program)]
    pub share_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(mut, token::mint = usdc_mint, token::token_program = usdc_token_program)]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    #[account(mut, token::mint = usdc_mint, token::token_program = usdc_token_program)]
    pub custodian: InterfaceAccount<'info, TokenAccount>,

    pub usdc_token_program: Interface<'info, TokenInterface>,
    pub share_token_program: Program<'info, Token2022>,
}

impl<'info> SweepToCustodian<'info> {
    pub fn sweep_to_custodian(&mut self, amount: u64) -> Result<()> {
        require_gt!(amount, 0, ErrorCode::InvalidAmount);

        let remaining = self
            .vault
            .amount
            .checked_sub(amount)
            .ok_or(ErrorCode::InsufficientLiquidity)?;
        let floor = self.fund.required_buffer(self.share_mint.supply)?;
        require!(remaining >= floor, ErrorCode::BufferFloorBreached);

        let seeds: &[&[&[u8]]] = &[&[FUND_SEED, &[self.fund.bump]]];
        transfer_checked(
            CpiContext::new_with_signer(
                self.usdc_token_program.key(),
                TransferChecked {
                    from: self.vault.to_account_info(),
                    mint: self.usdc_mint.to_account_info(),
                    to: self.custodian.to_account_info(),
                    authority: self.fund.to_account_info(),
                },
                seeds,
            ),
            amount,
            self.usdc_mint.decimals,
        )
    }
}
