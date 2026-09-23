use anchor_lang::{prelude::*, solana_program::program::invoke_signed};
use anchor_spl::{
    token_2022::spl_token_2022::extension::scaled_ui_amount,
    token_interface::{Mint, Token2022},
};

use crate::{
    error::ErrorCode, mul_div, Fund, Role, RoleType, BPS_DENOMINATOR, FUND_SEED, NAV_SCALE,
};

#[event_cpi]
#[derive(Accounts)]
pub struct PublishNav<'info> {
    pub nav_manager: Signer<'info>,

    #[account(
        seeds = [RoleType::NAV_MANAGER_SEED, nav_manager.key().as_ref()],
        bump = nav_manager_role.bump,
    )]
    pub nav_manager_role: Account<'info, Role>,

    #[account(mut, seeds = [FUND_SEED], bump = fund.bump, has_one = share_mint)]
    pub fund: Box<Account<'info, Fund>>,

    #[account(mut, mint::token_program = share_token_program)]
    pub share_mint: Box<InterfaceAccount<'info, Mint>>,

    pub share_token_program: Program<'info, Token2022>,
}

impl<'info> PublishNav<'info> {
    pub fn publish_nav(&mut self, new_nav: u64) -> Result<(u64, i64)> {
        require_gt!(new_nav, 0, ErrorCode::InvalidAmount);

        let old_nav = self.fund.nav;
        let max_move = mul_div(old_nav, self.fund.max_nav_change_bps, BPS_DENOMINATOR)?;
        require!(
            new_nav.abs_diff(old_nav) <= max_move,
            ErrorCode::NavDeviationTooLarge
        );

        let now = Clock::get()?.unix_timestamp;
        self.fund.nav = new_nav;
        self.fund.nav_updated_at = now;
        self.sync_ui_multiplier(new_nav, now)?;

        msg!("NAV {} -> {}", old_nav, new_nav);
        Ok((old_nav, now))
    }

    fn sync_ui_multiplier(&self, new_nav: u64, now: i64) -> Result<()> {
        let multiplier = new_nav as f64 / NAV_SCALE as f64;
        let seeds: &[&[&[u8]]] = &[&[FUND_SEED, &[self.fund.bump]]];
        invoke_signed(
            &scaled_ui_amount::instruction::update_multiplier(
                &self.share_token_program.key(),
                &self.share_mint.key(),
                &self.fund.key(),
                &[],
                multiplier,
                now,
            )?,
            &[
                self.share_mint.to_account_info(),
                self.fund.to_account_info(),
            ],
            seeds,
        )
        .map_err(Into::into)
    }
}
