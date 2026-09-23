use anchor_lang::prelude::*;

use crate::{error::ErrorCode, Fund, Role, RoleType, BPS_DENOMINATOR, FUND_SEED};

#[derive(Accounts)]
pub struct AdminConfig<'info> {
    pub admin: Signer<'info>,

    #[account(
        seeds = [RoleType::FUND_ADMIN_SEED, admin.key().as_ref()],
        bump = admin_role.bump,
    )]
    pub admin_role: Account<'info, Role>,

    #[account(mut, seeds = [FUND_SEED], bump = fund.bump)]
    pub fund: Account<'info, Fund>,
}

impl<'info> AdminConfig<'info> {
    pub fn resume(&mut self, subscriptions: bool, redemptions: bool) -> Result<()> {
        self.fund.subscriptions_paused &= !subscriptions;
        self.fund.redemptions_paused &= !redemptions;
        Ok(())
    }

    pub fn set_min_buffer_bps(&mut self, min_buffer_bps: u64) -> Result<()> {
        require!(min_buffer_bps <= BPS_DENOMINATOR, ErrorCode::InvalidParams);
        self.fund.min_buffer_bps = min_buffer_bps;
        Ok(())
    }

    pub fn set_nav_params(&mut self, max_nav_age: i64, max_nav_change_bps: u64) -> Result<()> {
        require_gt!(max_nav_age, 0, ErrorCode::InvalidParams);
        require!(
            max_nav_change_bps < BPS_DENOMINATOR,
            ErrorCode::InvalidParams
        );
        self.fund.max_nav_age = max_nav_age;
        self.fund.max_nav_change_bps = max_nav_change_bps;
        Ok(())
    }

    pub fn set_redemption_gate(&mut self, cap: u64, window_seconds: i64) -> Result<()> {
        require_gt!(cap, 0, ErrorCode::InvalidParams);
        require_gt!(window_seconds, 0, ErrorCode::InvalidParams);
        self.fund.redemption_cap = cap;
        self.fund.redemption_window = window_seconds;
        Ok(())
    }

    pub fn set_usdc_oracle(
        &mut self,
        enabled: bool,
        price_update: Pubkey,
        feed_id: [u8; 32],
        max_age: u64,
        max_deviation_bps: u64,
    ) -> Result<()> {
        require!(
            max_deviation_bps < BPS_DENOMINATOR,
            ErrorCode::InvalidParams
        );
        if enabled {
            require_keys_neq!(price_update, Pubkey::default(), ErrorCode::InvalidParams);
            require_gt!(max_age, 0, ErrorCode::InvalidParams);
            require_gt!(max_deviation_bps, 0, ErrorCode::InvalidParams);
        }

        self.fund.oracle_enabled = enabled;
        self.fund.usdc_price_update = price_update;
        self.fund.usdc_feed_id = feed_id;
        self.fund.usdc_max_age = max_age;
        self.fund.usdc_max_deviation_bps = max_deviation_bps;
        Ok(())
    }
}
