pub mod constants;
pub mod error;
pub mod events;
pub mod instructions;
pub mod state;
pub mod utils;

use anchor_lang::prelude::*;

pub use constants::*;
pub use events::*;
pub use instructions::*;
pub use state::*;
pub use utils::*;

declare_id!("BZVm6He91MdutTeAQFYqG8SxPt84vfkGwJULmAwMnb2E");

#[program]
pub mod tmmf {
    use super::*;

    #[instruction(discriminator = 0)]
    pub fn initialize_fund(ctx: Context<InitializeFund>, params: FundParams) -> Result<()> {
        ctx.accounts.initialize_fund(params, &ctx.bumps)
    }

    #[instruction(discriminator = 1)]
    pub fn approve_investor(ctx: Context<ApproveInvestor>) -> Result<()> {
        ctx.accounts.approve_investor()
    }

    #[instruction(discriminator = 2)]
    pub fn revoke_investor(ctx: Context<RevokeInvestor>) -> Result<()> {
        ctx.accounts.revoke_investor()
    }

    #[instruction(discriminator = 3)]
    pub fn subscribe(ctx: Context<Subscribe>, usdc_amount: u64) -> Result<()> {
        let shares_out = ctx.accounts.subscribe(usdc_amount)?;

        emit_cpi!(Subscribed {
            fund: ctx.accounts.fund.key(),
            investor: ctx.accounts.investor.key(),
            usdc_in: usdc_amount,
            shares_out,
            nav: ctx.accounts.fund.nav,
        });
        Ok(())
    }

    #[instruction(discriminator = 4)]
    pub fn redeem(ctx: Context<Redeem>, shares: u64) -> Result<()> {
        let usdc_out = ctx.accounts.redeem(shares)?;

        emit_cpi!(Redeemed {
            fund: ctx.accounts.fund.key(),
            investor: ctx.accounts.investor.key(),
            shares_in: shares,
            usdc_out,
            nav: ctx.accounts.fund.nav,
        });
        Ok(())
    }

    #[instruction(discriminator = 5)]
    pub fn publish_nav(ctx: Context<PublishNav>, new_nav: u64) -> Result<()> {
        let (old_nav, timestamp) = ctx.accounts.publish_nav(new_nav)?;

        emit_cpi!(NavPublished {
            fund: ctx.accounts.fund.key(),
            old_nav,
            new_nav,
            timestamp,
        });
        Ok(())
    }

    #[instruction(discriminator = 6)]
    pub fn sweep_to_custodian(ctx: Context<SweepToCustodian>, amount: u64) -> Result<()> {
        ctx.accounts.sweep_to_custodian(amount)
    }

    #[instruction(discriminator = 7)]
    pub fn pause(ctx: Context<PauseDealing>, subscriptions: bool, redemptions: bool) -> Result<()> {
        ctx.accounts.pause(subscriptions, redemptions)
    }

    #[instruction(discriminator = 8)]
    pub fn set_redemption_gate(
        ctx: Context<AdminConfig>,
        cap: u64,
        window_seconds: i64,
    ) -> Result<()> {
        ctx.accounts.set_redemption_gate(cap, window_seconds)
    }

    #[instruction(discriminator = 9)]
    pub fn set_min_buffer_bps(ctx: Context<AdminConfig>, min_buffer_bps: u64) -> Result<()> {
        ctx.accounts.set_min_buffer_bps(min_buffer_bps)
    }

    #[instruction(discriminator = 10)]
    pub fn resume(ctx: Context<AdminConfig>, subscriptions: bool, redemptions: bool) -> Result<()> {
        ctx.accounts.resume(subscriptions, redemptions)
    }

    #[instruction(discriminator = 11)]
    pub fn grant_role(ctx: Context<GrantRole>, role: RoleType, user: Pubkey) -> Result<()> {
        ctx.accounts.grant_role(role, user, ctx.bumps.role_to_grant)
    }

    #[instruction(discriminator = 12)]
    pub fn revoke_role(ctx: Context<RevokeRole>, role: RoleType, user: Pubkey) -> Result<()> {
        ctx.accounts.revoke_role(role, user)
    }

    #[instruction(discriminator = 14)]
    pub fn set_custodian(ctx: Context<SetCustodian>) -> Result<()> {
        ctx.accounts.set_custodian()
    }

    #[instruction(discriminator = 15)]
    pub fn set_nav_params(
        ctx: Context<AdminConfig>,
        max_nav_age: i64,
        max_nav_change_bps: u64,
    ) -> Result<()> {
        ctx.accounts.set_nav_params(max_nav_age, max_nav_change_bps)
    }

    #[instruction(discriminator = 13)]
    pub fn set_usdc_oracle(
        ctx: Context<AdminConfig>,
        enabled: bool,
        price_update: Pubkey,
        feed_id: [u8; 32],
        max_age: u64,
        max_deviation_bps: u64,
    ) -> Result<()> {
        ctx.accounts
            .set_usdc_oracle(enabled, price_update, feed_id, max_age, max_deviation_bps)
    }
}
