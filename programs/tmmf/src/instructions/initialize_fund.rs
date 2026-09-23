use anchor_lang::{
    prelude::*,
    solana_program::program::invoke,
    system_program::{
        allocate, assign, create_account, transfer, Allocate, Assign, CreateAccount, Transfer,
    },
};
use anchor_spl::{
    associated_token::AssociatedToken,
    token_2022::spl_token_2022::{
        extension::{scaled_ui_amount, ExtensionType},
        state::{AccountState, Mint as MintState},
    },
    token_interface::{
        default_account_state_initialize, initialize_mint2, DefaultAccountStateInitialize,
        InitializeMint2, Mint, Token2022, TokenAccount, TokenInterface,
    },
};

use crate::{
    error::ErrorCode, require_own_program_data, Fund, BPS_DENOMINATOR, FUND_SEED, NAV_SCALE,
    SHARE_MINT_SEED,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct FundParams {
    pub max_nav_age: i64,
    pub max_nav_change_bps: u64,
    pub min_buffer_bps: u64,
    pub redemption_cap: u64,
    pub redemption_window: i64,
}

#[derive(Accounts)]
pub struct InitializeFund<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(mint::token_program = usdc_token_program)]
    pub usdc_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        init,
        payer = admin,
        seeds = [FUND_SEED],
        space = Fund::DISCRIMINATOR.len() + Fund::INIT_SPACE,
        bump
    )]
    pub fund: Box<Account<'info, Fund>>,

    /// CHECK: see the README for why this account is unchecked.
    #[account(mut, seeds = [SHARE_MINT_SEED, fund.key().as_ref()], bump)]
    pub share_mint: UncheckedAccount<'info>,

    #[account(
        init,
        payer = admin,
        associated_token::mint = usdc_mint,
        associated_token::authority = fund,
        associated_token::token_program = usdc_token_program
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(token::mint = usdc_mint, token::token_program = usdc_token_program)]
    pub custodian: Box<InterfaceAccount<'info, TokenAccount>>,

    pub usdc_token_program: Interface<'info, TokenInterface>,
    pub share_token_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,

    #[account(address = crate::ID)]
    pub program: Program<'info, crate::program::Tmmf>,

    #[account(
        constraint =
            program_data.upgrade_authority_address == Some(admin.key()) @ ErrorCode::NotUpgradeAuthority
    )]
    pub program_data: Account<'info, ProgramData>,
}

impl<'info> InitializeFund<'info> {
    pub fn initialize_fund(
        &mut self,
        params: FundParams,
        bumps: &InitializeFundBumps,
    ) -> Result<()> {
        require!(
            params.max_nav_age > 0
                && params.redemption_window > 0
                && params.max_nav_change_bps < BPS_DENOMINATOR
                && params.min_buffer_bps <= BPS_DENOMINATOR,
            ErrorCode::InvalidParams
        );

        require_own_program_data(&self.program, &self.program_data)?;

        let now = Clock::get()?.unix_timestamp;
        self.fund.set_inner(Fund {
            share_mint: self.share_mint.key(),
            usdc_mint: self.usdc_mint.key(),
            vault: self.vault.key(),
            custodian: self.custodian.key(),
            decimals: self.usdc_mint.decimals,
            nav: NAV_SCALE,
            nav_updated_at: now,
            max_nav_age: params.max_nav_age,
            max_nav_change_bps: params.max_nav_change_bps,
            min_buffer_bps: params.min_buffer_bps,
            oracle_enabled: false,
            usdc_price_update: Pubkey::default(),
            usdc_feed_id: [0u8; 32],
            usdc_max_age: 0,
            usdc_max_deviation_bps: 0,
            subscriptions_paused: false,
            redemptions_paused: false,
            redemption_cap: params.redemption_cap,
            redemption_window: params.redemption_window,
            window_start: now,
            redeemed_in_window: 0,
            bump: bumps.fund,
        });

        self.create_share_mint(bumps.share_mint)
    }

    fn create_share_mint(&self, share_mint_bump: u8) -> Result<()> {
        let token_program_id = self.share_token_program.key();
        let fund = self.fund.key();

        let space = ExtensionType::try_calculate_account_len::<MintState>(&[
            ExtensionType::ScaledUiAmount,
            ExtensionType::DefaultAccountState,
        ])?;
        let signer_seeds: &[&[&[u8]]] = &[&[SHARE_MINT_SEED, fund.as_ref(), &[share_mint_bump]]];
        self.create_pda_account(space, &token_program_id, signer_seeds)?;

        invoke(
            &scaled_ui_amount::instruction::initialize(
                &token_program_id,
                &self.share_mint.key(),
                Some(fund),
                1.0,
            )?,
            &[self.share_mint.to_account_info()],
        )?;
        default_account_state_initialize(
            CpiContext::new(
                token_program_id,
                DefaultAccountStateInitialize {
                    token_program_id: self.share_token_program.to_account_info(),
                    mint: self.share_mint.to_account_info(),
                },
            ),
            &AccountState::Frozen,
        )?;
        initialize_mint2(
            CpiContext::new(
                token_program_id,
                InitializeMint2 {
                    mint: self.share_mint.to_account_info(),
                },
            ),
            self.usdc_mint.decimals,
            &fund,
            Some(&fund),
        )
    }

    fn create_pda_account(
        &self,
        space: usize,
        owner: &Pubkey,
        signer_seeds: &[&[&[u8]]],
    ) -> Result<()> {
        let system_program = self.system_program.key();
        let target = self.share_mint.to_account_info();
        let rent = Rent::get()?.minimum_balance(space);
        let current = target.lamports();

        if current == 0 {
            return create_account(
                CpiContext::new_with_signer(
                    system_program,
                    CreateAccount {
                        from: self.admin.to_account_info(),
                        to: target,
                    },
                    signer_seeds,
                ),
                rent,
                space as u64,
                owner,
            );
        }

        let top_up = rent.saturating_sub(current);
        if top_up > 0 {
            transfer(
                CpiContext::new(
                    system_program,
                    Transfer {
                        from: self.admin.to_account_info(),
                        to: target.clone(),
                    },
                ),
                top_up,
            )?;
        }
        allocate(
            CpiContext::new_with_signer(
                system_program,
                Allocate {
                    account_to_allocate: target.clone(),
                },
                signer_seeds,
            ),
            space as u64,
        )?;
        assign(
            CpiContext::new_with_signer(
                system_program,
                Assign {
                    account_to_assign: target,
                },
                signer_seeds,
            ),
            owner,
        )
    }
}
