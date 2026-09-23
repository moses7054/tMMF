use anchor_lang::prelude::*;
use pyth_solana_receiver_sdk::price_update::PriceUpdateV2;

use crate::{error::ErrorCode, BPS_DENOMINATOR, NAV_SCALE};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum RoleType {
    FundAdmin,
    NavManager,
    Pauser,
}

impl RoleType {
    pub const FUND_ADMIN_SEED: &'static [u8] = b"role_fund_admin";
    pub const NAV_MANAGER_SEED: &'static [u8] = b"role_nav_manager";
    pub const PAUSER_SEED: &'static [u8] = b"role_pauser";

    pub fn seed(&self) -> &'static [u8] {
        match self {
            RoleType::FundAdmin => Self::FUND_ADMIN_SEED,
            RoleType::NavManager => Self::NAV_MANAGER_SEED,
            RoleType::Pauser => Self::PAUSER_SEED,
        }
    }
}

#[account]
#[derive(InitSpace)]
pub struct Role {
    pub address: Pubkey,
    pub role: RoleType,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct Fund {
    pub share_mint: Pubkey,
    pub usdc_mint: Pubkey,
    pub vault: Pubkey,
    pub custodian: Pubkey,

    pub decimals: u8,

    pub nav: u64,
    pub nav_updated_at: i64,
    pub max_nav_age: i64,
    pub max_nav_change_bps: u64,

    pub subscriptions_paused: bool,
    pub redemptions_paused: bool,

    pub oracle_enabled: bool,
    pub usdc_price_update: Pubkey,
    pub usdc_feed_id: [u8; 32],
    pub usdc_max_age: u64,
    pub usdc_max_deviation_bps: u64,

    pub min_buffer_bps: u64,

    pub redemption_cap: u64,
    pub redemption_window: i64,
    pub window_start: i64,
    pub redeemed_in_window: u64,

    pub bump: u8,
}

impl Fund {
    pub fn usdc_to_shares(&self, usdc: u64) -> Result<u64> {
        mul_div(usdc, NAV_SCALE, self.nav)
    }

    pub fn shares_to_usdc(&self, shares: u64) -> Result<u64> {
        mul_div(shares, self.nav, NAV_SCALE)
    }

    pub fn required_buffer(&self, shares_outstanding: u64) -> Result<u64> {
        let aum = self.shares_to_usdc(shares_outstanding)?;
        mul_div(aum, self.min_buffer_bps, BPS_DENOMINATOR)
    }

    pub fn require_usdc_at_par(&self, price: i64, exponent: i32) -> Result<()> {
        require_gt!(price, 0, ErrorCode::InvalidOraclePrice);

        let scaled = match exponent {
            e if e < 0 => {
                let divisor = 10i128
                    .checked_pow(e.unsigned_abs())
                    .ok_or(ErrorCode::MathOverflow)?;
                (price as i128)
                    .checked_mul(NAV_SCALE as i128)
                    .and_then(|v| v.checked_div(divisor))
                    .ok_or(ErrorCode::MathOverflow)?
            }
            e => {
                let multiplier = 10i128
                    .checked_pow(e as u32)
                    .ok_or(ErrorCode::MathOverflow)?;
                (price as i128)
                    .checked_mul(NAV_SCALE as i128)
                    .and_then(|v| v.checked_mul(multiplier))
                    .ok_or(ErrorCode::MathOverflow)?
            }
        };

        let par = NAV_SCALE as i128;
        let allowed = par
            .checked_mul(self.usdc_max_deviation_bps as i128)
            .and_then(|v| v.checked_div(BPS_DENOMINATOR as i128))
            .ok_or(ErrorCode::MathOverflow)?;
        require!((scaled - par).abs() <= allowed, ErrorCode::UsdcDepegged);
        Ok(())
    }

    pub fn check_usdc_peg(&self, price_update: Option<&Account<PriceUpdateV2>>) -> Result<()> {
        if !self.oracle_enabled {
            return Ok(());
        }
        let update = price_update.ok_or(ErrorCode::OracleRequired)?;
        require_keys_eq!(
            update.key(),
            self.usdc_price_update,
            ErrorCode::OracleRequired
        );

        let price = update
            .get_price_no_older_than(&Clock::get()?, self.usdc_max_age, &self.usdc_feed_id)
            .map_err(|_| ErrorCode::OracleUnavailable)?;

        self.require_usdc_at_par(price.price, price.exponent)
    }

    pub fn require_fresh_nav(&self, now: i64) -> Result<()> {
        require!(
            now.saturating_sub(self.nav_updated_at) <= self.max_nav_age,
            ErrorCode::StaleNav
        );
        Ok(())
    }

    pub fn consume_redemption_capacity(&mut self, usdc: u64, now: i64) -> Result<()> {
        if now.saturating_sub(self.window_start) >= self.redemption_window {
            self.window_start = now;
            self.redeemed_in_window = 0;
        }
        let used = self
            .redeemed_in_window
            .checked_add(usdc)
            .ok_or(ErrorCode::MathOverflow)?;
        require!(
            used <= self.redemption_cap,
            ErrorCode::RedemptionGateExceeded
        );
        self.redeemed_in_window = used;
        Ok(())
    }
}

pub fn mul_div(a: u64, b: u64, c: u64) -> Result<u64> {
    let r = (a as u128)
        .checked_mul(b as u128)
        .and_then(|x| x.checked_div(c as u128))
        .ok_or(ErrorCode::MathOverflow)?;
    u64::try_from(r).map_err(|_| ErrorCode::MathOverflow.into())
}
