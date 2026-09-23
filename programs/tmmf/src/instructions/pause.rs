use anchor_lang::prelude::*;

use crate::{Fund, Role, RoleType, FUND_SEED};

#[derive(Accounts)]
pub struct PauseDealing<'info> {
    pub pauser: Signer<'info>,

    #[account(
        seeds = [RoleType::PAUSER_SEED, pauser.key().as_ref()],
        bump = pauser_role.bump,
    )]
    pub pauser_role: Account<'info, Role>,

    #[account(mut, seeds = [FUND_SEED], bump = fund.bump)]
    pub fund: Account<'info, Fund>,
}

impl<'info> PauseDealing<'info> {
    pub fn pause(&mut self, subscriptions: bool, redemptions: bool) -> Result<()> {
        self.fund.subscriptions_paused |= subscriptions;
        self.fund.redemptions_paused |= redemptions;
        Ok(())
    }
}
