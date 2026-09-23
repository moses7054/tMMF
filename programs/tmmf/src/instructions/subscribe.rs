use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    mint_to_checked, transfer_checked, MintToChecked, Token2022, TokenAccount, TokenInterface,
    TransferChecked,
};
use pyth_solana_receiver_sdk::price_update::PriceUpdateV2;

use crate::{error::ErrorCode, Fund, FUND_SEED};

#[event_cpi]
#[derive(Accounts)]
pub struct Subscribe<'info> {
    pub investor: Signer<'info>,

    #[account(
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

impl<'info> Subscribe<'info> {
    pub fn subscribe(&self, usdc_amount: u64) -> Result<u64> {
        require!(
            !self.fund.subscriptions_paused,
            ErrorCode::SubscriptionsPaused
        );
        require_gt!(usdc_amount, 0, ErrorCode::InvalidAmount);
        self.fund.require_fresh_nav(Clock::get()?.unix_timestamp)?;
        self.fund.check_usdc_peg(self.usdc_price_update.as_ref())?;

        let shares = self.fund.usdc_to_shares(usdc_amount)?;
        require_gt!(shares, 0, ErrorCode::InvalidAmount);

        self.collect_usdc(usdc_amount)?;
        self.mint_shares(shares)?;

        Ok(shares)
    }

    fn collect_usdc(&self, usdc_amount: u64) -> Result<()> {
        transfer_checked(
            CpiContext::new(
                self.usdc_token_program.key(),
                TransferChecked {
                    from: self.investor_usdc.to_account_info(),
                    mint: self.usdc_mint.to_account_info(),
                    to: self.vault.to_account_info(),
                    authority: self.investor.to_account_info(),
                },
            ),
            usdc_amount,
            self.fund.decimals,
        )
    }

    fn mint_shares(&self, shares: u64) -> Result<()> {
        let seeds: &[&[&[u8]]] = &[&[FUND_SEED, &[self.fund.bump]]];
        mint_to_checked(
            CpiContext::new_with_signer(
                self.share_token_program.key(),
                MintToChecked {
                    mint: self.share_mint.to_account_info(),
                    to: self.investor_shares.to_account_info(),
                    authority: self.fund.to_account_info(),
                },
                seeds,
            ),
            shares,
            self.fund.decimals,
        )
    }
}
