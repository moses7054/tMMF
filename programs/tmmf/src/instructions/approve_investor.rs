use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{thaw_account, Mint, ThawAccount, Token2022, TokenAccount},
};

use crate::{events::InvestorStatusChanged, Fund, Role, RoleType, FUND_SEED};

#[derive(Accounts)]
pub struct ApproveInvestor<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        seeds = [RoleType::FUND_ADMIN_SEED, admin.key().as_ref()],
        bump = admin_role.bump,
    )]
    pub admin_role: Account<'info, Role>,

    #[account(seeds = [FUND_SEED], bump = fund.bump, has_one = share_mint)]
    pub fund: Box<Account<'info, Fund>>,

    /// CHECK: see the README for why this account is unchecked.
    pub investor: UncheckedAccount<'info>,

    #[account(mint::token_program = share_token_program)]
    pub share_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        init_if_needed,
        payer = admin,
        associated_token::mint = share_mint,
        associated_token::authority = investor,
        associated_token::token_program = share_token_program
    )]
    pub investor_shares: Box<InterfaceAccount<'info, TokenAccount>>,

    pub share_token_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

impl<'info> ApproveInvestor<'info> {
    pub fn approve_investor(&mut self) -> Result<()> {
        if !self.investor_shares.is_frozen() {
            return Ok(());
        }

        let seeds: &[&[&[u8]]] = &[&[FUND_SEED, &[self.fund.bump]]];
        thaw_account(CpiContext::new_with_signer(
            self.share_token_program.key(),
            ThawAccount {
                account: self.investor_shares.to_account_info(),
                mint: self.share_mint.to_account_info(),
                authority: self.fund.to_account_info(),
            },
            seeds,
        ))?;

        emit!(InvestorStatusChanged {
            fund: self.fund.key(),
            investor: self.investor.key(),
            approved: true,
        });
        Ok(())
    }
}
