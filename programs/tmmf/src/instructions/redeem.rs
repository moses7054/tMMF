use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    burn_checked, transfer_checked, BurnChecked, Token2022, TokenAccount, TokenInterface,
    TransferChecked,
};
use pyth_solana_receiver_sdk::price_update::PriceUpdateV2;

use crate::{error::ErrorCode, Fund, FUND_SEED};

#[event_cpi]
#[derive(Accounts)]
pub struct Redeem<'info> {
    pub investor: Signer<'info>,

    #[account(
        mut,
        seeds = [FUND_SEED],
        bump = fund.bump,
        has_one = share_mint,
        has_one = usdc_mint,
        has_one = vault,
    )]
    pub fund: Box<Account<'info, Fund>>,

    /// CHECK: see the README for why this account is unchecked.
    #[account(mut)]
    pub share_mint: UncheckedAccount<'info>,

    /// CHECK: see the README for why this account is unchecked.
    pub usdc_mint: UncheckedAccount<'info>,

    #[account(
        mut,
        token::mint = usdc_mint,
        token::authority = investor,
        token::token_program = usdc_token_program
    )]
    pub investor_usdc: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        token::mint = share_mint,
        token::authority = investor,
        token::token_program = share_token_program
    )]
    pub investor_shares: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(mut, token::mint = usdc_mint, token::token_program = usdc_token_program)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    pub usdc_price_update: Option<Account<'info, PriceUpdateV2>>,

    pub usdc_token_program: Interface<'info, TokenInterface>,
    pub share_token_program: Program<'info, Token2022>,
}

impl<'info> Redeem<'info> {
    pub fn redeem(&mut self, shares: u64) -> Result<u64> {
        require!(!self.fund.redemptions_paused, ErrorCode::RedemptionsPaused);
        require_gt!(shares, 0, ErrorCode::InvalidAmount);
        let now = Clock::get()?.unix_timestamp;
        self.fund.require_fresh_nav(now)?;
        self.fund.check_usdc_peg(self.usdc_price_update.as_ref())?;

        let usdc_out = self.fund.shares_to_usdc(shares)?;
        require_gt!(usdc_out, 0, ErrorCode::InvalidAmount);

        require!(
            usdc_out <= self.vault.amount,
            ErrorCode::InsufficientLiquidity
        );
        self.fund.consume_redemption_capacity(usdc_out, now)?;

        self.burn_shares(shares)?;
        self.pay_out(usdc_out)?;

        Ok(usdc_out)
    }

    fn burn_shares(&self, shares: u64) -> Result<()> {
        burn_checked(
            CpiContext::new(
                self.share_token_program.key(),
                BurnChecked {
                    mint: self.share_mint.to_account_info(),
                    from: self.investor_shares.to_account_info(),
                    authority: self.investor.to_account_info(),
                },
            ),
            shares,
            self.fund.decimals,
        )
    }

    fn pay_out(&self, usdc_out: u64) -> Result<()> {
        let seeds: &[&[&[u8]]] = &[&[FUND_SEED, &[self.fund.bump]]];
        transfer_checked(
            CpiContext::new_with_signer(
                self.usdc_token_program.key(),
                TransferChecked {
                    from: self.vault.to_account_info(),
                    mint: self.usdc_mint.to_account_info(),
                    to: self.investor_usdc.to_account_info(),
                    authority: self.fund.to_account_info(),
                },
                seeds,
            ),
            usdc_out,
            self.fund.decimals,
        )
    }
}
