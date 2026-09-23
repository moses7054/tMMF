use anchor_lang::prelude::*;
use anchor_spl::token_interface::{freeze_account, FreezeAccount, Mint, Token2022, TokenAccount};

use crate::{events::InvestorStatusChanged, Fund, Role, RoleType, FUND_SEED};

#[derive(Accounts)]
pub struct RevokeInvestor<'info> {
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
        mut,
        associated_token::mint = share_mint,
        associated_token::authority = investor,
        associated_token::token_program = share_token_program
    )]
    pub investor_shares: Box<InterfaceAccount<'info, TokenAccount>>,

    pub share_token_program: Program<'info, Token2022>,
}

impl<'info> RevokeInvestor<'info> {
    pub fn revoke_investor(&mut self) -> Result<()> {
        if self.investor_shares.is_frozen() {
            return Ok(());
        }

        let seeds: &[&[&[u8]]] = &[&[FUND_SEED, &[self.fund.bump]]];
        freeze_account(CpiContext::new_with_signer(
            self.share_token_program.key(),
            FreezeAccount {
                account: self.investor_shares.to_account_info(),
                mint: self.share_mint.to_account_info(),
                authority: self.fund.to_account_info(),
            },
            seeds,
        ))?;

        emit!(InvestorStatusChanged {
            fund: self.fund.key(),
            investor: self.investor.key(),
            approved: false,
        });
        Ok(())
    }
}
