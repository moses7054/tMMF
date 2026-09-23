use anchor_lang::{
    prelude::Clock, solana_program::bpf_loader_upgradeable::get_program_data_address,
    system_program, AccountDeserialize, AccountSerialize, InstructionData, ToAccountMetas,
};
use anchor_spl::{
    associated_token::{self, get_associated_token_address_with_program_id},
    token_2022::spl_token_2022::{
        self,
        extension::{
            scaled_ui_amount::ScaledUiAmountConfig, BaseStateWithExtensions, StateWithExtensions,
        },
        state::{Account as TokenAccount2022, Mint as Mint2022},
    },
};
use litesvm::LiteSVM;
use litesvm_token::{spl_token, CreateAssociatedTokenAccount, CreateMint, MintTo};
use pyth_solana_receiver_sdk::price_update::{PriceFeedMessage, PriceUpdateV2, VerificationLevel};
use solana_keypair::Keypair;
use solana_message::{
    v1::{self, TransactionConfig},
    Instruction, Message, VersionedMessage,
};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;
use tmmf::{FundParams, RoleType, FUND_SEED, NAV_SCALE, SHARE_MINT_SEED};

const USDC: u64 = 1_000_000;
const DAY: i64 = 86_400;

const LEGACY_MAX_TX_BYTES: usize = 1_232;

#[derive(Clone, Copy)]
enum TxFormat {
    Legacy,
    V1,
}

fn v1_config() -> TransactionConfig {
    TransactionConfig::empty()
        .with_compute_unit_limit(1_400_000)
        .with_loaded_accounts_data_size_limit(64 * 1024 * 1024)
}

fn build_tx(
    format: TxFormat,
    ixs: &[Instruction],
    signers: &[&Keypair],
    blockhash: solana_message::Hash,
) -> VersionedTransaction {
    let payer = signers[0].pubkey();
    let msg = match format {
        TxFormat::Legacy => {
            VersionedMessage::Legacy(Message::new_with_blockhash(ixs, Some(&payer), &blockhash))
        }
        TxFormat::V1 => VersionedMessage::V1(
            v1::Message::try_compile_with_config(&payer, ixs, blockhash, v1_config()).unwrap(),
        ),
    };
    VersionedTransaction::try_new(msg, signers).unwrap()
}

fn wire_size(tx: &VersionedTransaction) -> usize {
    let prefix = match tx.message {
        VersionedMessage::V1(_) => 0,
        _ => 1,
    };
    prefix + 64 * tx.signatures.len() + tx.message.serialize().len()
}

struct Env {
    svm: LiteSVM,
    admin: Keypair,
    nav_manager: Keypair,
    usdc_mint: Pubkey,
    fund: Pubkey,
    share_mint: Pubkey,
    vault: Pubkey,
    custodian: Pubkey,
    last_cu: u64,
    init_cu: u64,
    usdc_price_update: Option<Pubkey>,
}

impl Env {
    fn new() -> Self {
        Self::new_with(false)
    }

    fn new_with(prefund_share_mint: bool) -> Self {
        Self::new_seeded(prefund_share_mint, None)
    }

    fn new_seeded(prefund_share_mint: bool, seed: Option<u8>) -> Self {
        let mut svm = LiteSVM::new();
        let bytes = include_bytes!(concat!(env!("CARGO_TARGET_TMPDIR"), "/../deploy/tmmf.so"));
        svm.add_program(tmmf::id(), bytes).unwrap();

        let (admin, nav_manager) = match seed {
            Some(t) => (fixed_keypair(t), fixed_keypair(t.wrapping_add(1))),
            None => (Keypair::new(), Keypair::new()),
        };
        set_upgrade_authority(&mut svm, Some(admin.pubkey()));
        svm.airdrop(&admin.pubkey(), 100_000_000_000).unwrap();
        svm.airdrop(&nav_manager.pubkey(), 1_000_000_000).unwrap();

        let usdc_mint = CreateMint::new(&mut svm, &admin)
            .decimals(6)
            .authority(&admin.pubkey())
            .send()
            .unwrap();
        let custodian = CreateAssociatedTokenAccount::new(&mut svm, &admin, &usdc_mint)
            .owner(&admin.pubkey())
            .send()
            .unwrap();

        let (fund, _) = Pubkey::find_program_address(&[FUND_SEED], &tmmf::id());
        let (share_mint, _) =
            Pubkey::find_program_address(&[SHARE_MINT_SEED, fund.as_ref()], &tmmf::id());
        let vault =
            get_associated_token_address_with_program_id(&fund, &usdc_mint, &spl_token::id());

        let mut env = Env {
            svm,
            admin,
            nav_manager,
            usdc_mint,
            fund,
            share_mint,
            vault,
            custodian,
            last_cu: 0,
            init_cu: 0,
            usdc_price_update: None,
        };
        env.set_time(1_780_000_000);
        if prefund_share_mint {
            env.svm.airdrop(&share_mint, 1_000_000).unwrap();
        }

        let ix = Instruction {
            program_id: tmmf::id(),
            accounts: tmmf::accounts::InitializeFund {
                admin: env.admin.pubkey(),
                usdc_mint,
                fund,
                share_mint,
                vault,
                custodian,
                usdc_token_program: spl_token::id(),
                share_token_program: spl_token_2022::id(),
                associated_token_program: associated_token::ID,
                system_program: system_program::ID,
                program: tmmf::id(),
                program_data: get_program_data_address(&tmmf::id()),
            }
            .to_account_metas(None),
            data: tmmf::instruction::InitializeFund {
                params: FundParams {
                    max_nav_age: 3 * DAY,
                    max_nav_change_bps: 10,
                    min_buffer_bps: 2_500,
                    redemption_cap: 5_000 * USDC,
                    redemption_window: DAY,
                },
            }
            .data(),
        };
        let admin = env.admin.insecure_clone();
        env.send(ix, &[&admin]).unwrap();
        env.init_cu = env.last_cu;

        let nav_manager = env.nav_manager.pubkey();
        env.grant_role(RoleType::FundAdmin, &admin.pubkey())
            .unwrap();
        env.grant_role(RoleType::NavManager, &nav_manager).unwrap();
        env.grant_role(RoleType::Pauser, &admin.pubkey()).unwrap();
        env
    }

    fn grant_role(&mut self, role: RoleType, user: &Pubkey) -> Result<(), String> {
        let admin = self.admin.insecure_clone();
        let ix = Instruction {
            program_id: tmmf::id(),
            accounts: tmmf::accounts::GrantRole {
                payer: admin.pubkey(),
                authority: admin.pubkey(),
                role_to_grant: role_pda(role, user),
                program: tmmf::id(),
                program_data: get_program_data_address(&tmmf::id()),
                system_program: system_program::ID,
            }
            .to_account_metas(None),
            data: tmmf::instruction::GrantRole { role, user: *user }.data(),
        };
        self.send(ix, &[&admin])
    }

    fn revoke_role(&mut self, role: RoleType, user: &Pubkey) -> Result<(), String> {
        let admin = self.admin.insecure_clone();
        let ix = Instruction {
            program_id: tmmf::id(),
            accounts: tmmf::accounts::RevokeRole {
                rent_recipient: admin.pubkey(),
                authority: admin.pubkey(),
                role_to_revoke: role_pda(role, user),
                program: tmmf::id(),
                program_data: get_program_data_address(&tmmf::id()),
            }
            .to_account_metas(None),
            data: tmmf::instruction::RevokeRole { role, user: *user }.data(),
        };
        self.send(ix, &[&admin])
    }

    fn send(&mut self, ix: Instruction, signers: &[&Keypair]) -> Result<(), String> {
        let tx = self.tx(TxFormat::V1, &[ix], signers);
        self.send_tx(tx)
    }

    fn tx(
        &mut self,
        format: TxFormat,
        ixs: &[Instruction],
        signers: &[&Keypair],
    ) -> VersionedTransaction {
        self.svm.expire_blockhash();
        build_tx(format, ixs, signers, self.svm.latest_blockhash())
    }

    fn send_tx(&mut self, tx: VersionedTransaction) -> Result<(), String> {
        let res = self
            .svm
            .send_transaction(tx)
            .map(|m| m.compute_units_consumed)
            .map_err(|e| format!("{:?}\n{}", e.err, e.meta.pretty_logs()));
        if let Ok(cu) = res {
            self.last_cu = cu;
        }
        res.map(|_| ())
    }

    fn post_usdc_price(&mut self, price: i64, exponent: i32, publish_time: i64) {
        let feed_id = [7u8; 32];
        let addr = Pubkey::new_from_array([9u8; 32]);
        let update = PriceUpdateV2 {
            write_authority: Pubkey::default(),
            verification_level: VerificationLevel::Full,
            price_message: PriceFeedMessage {
                feed_id,
                price,
                conf: 1_000,
                exponent,
                publish_time,
                prev_publish_time: publish_time - 1,
                ema_price: price,
                ema_conf: 1_000,
            },
            posted_slot: 0,
        };
        let mut data = Vec::new();
        update.try_serialize(&mut data).unwrap();
        let lamports = self.svm.minimum_balance_for_rent_exemption(data.len());
        let account = solana_account::Account {
            lamports,
            data,
            owner: pyth_solana_receiver_sdk::ID,
            executable: false,
            rent_epoch: 0,
        };
        self.svm.set_account(addr, account).unwrap();
        self.usdc_price_update = Some(addr);
    }

    fn enable_usdc_guard(&mut self, max_age: u64, max_deviation_bps: u64) -> Result<(), String> {
        let price_update = self.usdc_price_update.expect("post a price first");
        self.admin_ix(
            tmmf::instruction::SetUsdcOracle {
                enabled: true,
                price_update,
                feed_id: [7u8; 32],
                max_age,
                max_deviation_bps,
            }
            .data(),
        )
    }

    fn set_time(&mut self, unix_timestamp: i64) {
        let mut clock: Clock = self.svm.get_sysvar();
        clock.unix_timestamp = unix_timestamp;
        self.svm.set_sysvar(&clock);
    }

    fn now(&self) -> i64 {
        self.svm.get_sysvar::<Clock>().unix_timestamp
    }

    fn fund(&self) -> tmmf::Fund {
        let acc = self.svm.get_account(&self.fund).unwrap();
        tmmf::Fund::try_deserialize(&mut acc.data.as_ref()).unwrap()
    }

    fn shares_ata(&self, owner: &Pubkey) -> Pubkey {
        get_associated_token_address_with_program_id(owner, &self.share_mint, &spl_token_2022::id())
    }

    fn usdc_ata(&self, owner: &Pubkey) -> Pubkey {
        get_associated_token_address_with_program_id(owner, &self.usdc_mint, &spl_token::id())
    }

    fn token_balance(&self, ata: &Pubkey) -> u64 {
        let acc = self.svm.get_account(ata).unwrap();
        StateWithExtensions::<TokenAccount2022>::unpack(&acc.data)
            .unwrap()
            .base
            .amount
    }

    fn ui_multiplier(&self) -> f64 {
        let acc = self.svm.get_account(&self.share_mint).unwrap();
        let mint = StateWithExtensions::<Mint2022>::unpack(&acc.data).unwrap();
        f64::from(
            mint.get_extension::<ScaledUiAmountConfig>()
                .unwrap()
                .new_multiplier,
        )
    }

    fn new_investor(&mut self, usdc: u64) -> Keypair {
        self.new_investor_seeded(usdc, None)
    }

    fn new_investor_seeded(&mut self, usdc: u64, seed: Option<u8>) -> Keypair {
        let investor = seed.map(fixed_keypair).unwrap_or_else(Keypair::new);
        self.svm.airdrop(&investor.pubkey(), 1_000_000_000).unwrap();
        let ata = CreateAssociatedTokenAccount::new(&mut self.svm, &investor, &self.usdc_mint)
            .owner(&investor.pubkey())
            .send()
            .unwrap();
        MintTo::new(&mut self.svm, &self.admin, &self.usdc_mint, &ata, usdc)
            .send()
            .unwrap();
        investor
    }

    fn set_investor_ix(&self, investor: &Pubkey, approve: bool) -> Instruction {
        let (accounts, data) = if approve {
            (
                tmmf::accounts::ApproveInvestor {
                    admin: self.admin.pubkey(),
                    admin_role: role_pda(RoleType::FundAdmin, &self.admin.pubkey()),
                    fund: self.fund,
                    investor: *investor,
                    share_mint: self.share_mint,
                    investor_shares: self.shares_ata(investor),
                    share_token_program: spl_token_2022::id(),
                    associated_token_program: associated_token::ID,
                    system_program: system_program::ID,
                }
                .to_account_metas(None),
                tmmf::instruction::ApproveInvestor {}.data(),
            )
        } else {
            (
                tmmf::accounts::RevokeInvestor {
                    admin: self.admin.pubkey(),
                    admin_role: role_pda(RoleType::FundAdmin, &self.admin.pubkey()),
                    fund: self.fund,
                    investor: *investor,
                    share_mint: self.share_mint,
                    investor_shares: self.shares_ata(investor),
                    share_token_program: spl_token_2022::id(),
                }
                .to_account_metas(None),
                tmmf::instruction::RevokeInvestor {}.data(),
            )
        };
        Instruction {
            program_id: tmmf::id(),
            accounts,
            data,
        }
    }

    fn approve(&mut self, investor: &Pubkey) {
        let ix = self.set_investor_ix(investor, true);
        let admin = self.admin.insecure_clone();
        self.send(ix, &[&admin]).unwrap();
    }

    fn subscribe(&mut self, investor: &Keypair, usdc_amount: u64) -> Result<(), String> {
        let ix = Instruction {
            program_id: tmmf::id(),
            accounts: tmmf::accounts::Subscribe {
                investor: investor.pubkey(),
                fund: self.fund,
                share_mint: self.share_mint,
                usdc_mint: self.usdc_mint,
                investor_usdc: self.usdc_ata(&investor.pubkey()),
                investor_shares: self.shares_ata(&investor.pubkey()),
                vault: self.vault,
                usdc_price_update: self.usdc_price_update,
                usdc_token_program: spl_token::id(),
                share_token_program: spl_token_2022::id(),
                event_authority: event_authority(),
                program: tmmf::id(),
            }
            .to_account_metas(None),
            data: tmmf::instruction::Subscribe { usdc_amount }.data(),
        };
        self.send(ix, &[investor])
    }

    fn redeem(&mut self, investor: &Keypair, shares: u64) -> Result<(), String> {
        let ix = Instruction {
            program_id: tmmf::id(),
            accounts: tmmf::accounts::Redeem {
                investor: investor.pubkey(),
                fund: self.fund,
                share_mint: self.share_mint,
                usdc_mint: self.usdc_mint,
                investor_usdc: self.usdc_ata(&investor.pubkey()),
                investor_shares: self.shares_ata(&investor.pubkey()),
                vault: self.vault,
                usdc_price_update: self.usdc_price_update,
                usdc_token_program: spl_token::id(),
                share_token_program: spl_token_2022::id(),
                event_authority: event_authority(),
                program: tmmf::id(),
            }
            .to_account_metas(None),
            data: tmmf::instruction::Redeem { shares }.data(),
        };
        self.send(ix, &[investor])
    }

    fn publish_nav(&mut self, new_nav: u64) -> Result<(), String> {
        let ix = Instruction {
            program_id: tmmf::id(),
            accounts: tmmf::accounts::PublishNav {
                nav_manager: self.nav_manager.pubkey(),
                nav_manager_role: role_pda(RoleType::NavManager, &self.nav_manager.pubkey()),
                fund: self.fund,
                share_mint: self.share_mint,
                share_token_program: spl_token_2022::id(),
                event_authority: event_authority(),
                program: tmmf::id(),
            }
            .to_account_metas(None),
            data: tmmf::instruction::PublishNav { new_nav }.data(),
        };
        let nav_manager = self.nav_manager.insecure_clone();
        self.send(ix, &[&nav_manager])
    }

    fn pause(&mut self, subscriptions: bool, redemptions: bool) -> Result<(), String> {
        let pauser = self.admin.insecure_clone();
        let ix = Instruction {
            program_id: tmmf::id(),
            accounts: tmmf::accounts::PauseDealing {
                pauser: pauser.pubkey(),
                pauser_role: role_pda(RoleType::Pauser, &pauser.pubkey()),
                fund: self.fund,
            }
            .to_account_metas(None),
            data: tmmf::instruction::Pause {
                subscriptions,
                redemptions,
            }
            .data(),
        };
        self.send(ix, &[&pauser])
    }

    fn resume(&mut self, subscriptions: bool, redemptions: bool) -> Result<(), String> {
        self.admin_ix(
            tmmf::instruction::Resume {
                subscriptions,
                redemptions,
            }
            .data(),
        )
    }

    fn admin_ix(&mut self, data: Vec<u8>) -> Result<(), String> {
        let ix = Instruction {
            program_id: tmmf::id(),
            accounts: tmmf::accounts::AdminConfig {
                admin: self.admin.pubkey(),
                admin_role: role_pda(RoleType::FundAdmin, &self.admin.pubkey()),
                fund: self.fund,
            }
            .to_account_metas(None),
            data,
        };
        let admin = self.admin.insecure_clone();
        self.send(ix, &[&admin])
    }

    fn sweep(&mut self, amount: u64) -> Result<(), String> {
        let ix = Instruction {
            program_id: tmmf::id(),
            accounts: tmmf::accounts::SweepToCustodian {
                admin: self.admin.pubkey(),
                admin_role: role_pda(RoleType::FundAdmin, &self.admin.pubkey()),
                fund: self.fund,
                usdc_mint: self.usdc_mint,
                vault: self.vault,
                share_mint: self.share_mint,
                custodian: self.custodian,
                usdc_token_program: spl_token::id(),
                share_token_program: spl_token_2022::id(),
            }
            .to_account_metas(None),
            data: tmmf::instruction::SweepToCustodian { amount }.data(),
        };
        let admin = self.admin.insecure_clone();
        self.send(ix, &[&admin])
    }

    fn transfer_shares(&mut self, from: &Keypair, to: &Pubkey, amount: u64) -> Result<(), String> {
        let ix = spl_token_2022::instruction::transfer_checked(
            &spl_token_2022::id(),
            &self.shares_ata(&from.pubkey()),
            &self.share_mint,
            &self.shares_ata(to),
            &from.pubkey(),
            &[],
            amount,
            6,
        )
        .unwrap();
        self.send(ix, &[from])
    }
}

fn fixed_keypair(tag: u8) -> Keypair {
    Keypair::new_from_array([tag; 32])
}

fn set_upgrade_authority(svm: &mut LiteSVM, authority: Option<Pubkey>) {
    let addr = get_program_data_address(&tmmf::id());
    let mut acc = svm.get_account(&addr).expect("programdata account");
    match authority {
        Some(key) => {
            acc.data[12] = 1;
            acc.data[13..45].copy_from_slice(key.as_ref());
        }
        None => {
            acc.data[12] = 0;
            acc.data[13..45].fill(0);
        }
    }
    svm.set_account(addr, acc).unwrap();
}

fn role_pda(role: RoleType, user: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[role.seed(), user.as_ref()], &tmmf::id()).0
}

fn event_authority() -> Pubkey {
    Pubkey::find_program_address(&[b"__event_authority"], &tmmf::id()).0
}

fn accrue_one_day(nav: u64) -> u64 {
    nav + nav * 500 / 10_000 / 365
}

#[test]
fn lifecycle_subscribe_accrue_redeem() {
    let mut env = Env::new();
    let alice = env.new_investor(1_000 * USDC);
    env.approve(&alice.pubkey());

    env.subscribe(&alice, 1_000 * USDC).unwrap();
    let alice_shares = env.shares_ata(&alice.pubkey());
    assert_eq!(env.token_balance(&alice_shares), 1_000 * USDC);
    assert_eq!(env.token_balance(&env.vault), 1_000 * USDC);

    let mut nav = NAV_SCALE;
    for _ in 0..3 {
        env.set_time(env.now() + DAY);
        nav = accrue_one_day(nav);
        env.publish_nav(nav).unwrap();
    }
    assert_eq!(env.fund().nav, nav);
    assert_eq!(env.token_balance(&alice_shares), 1_000 * USDC);
    let displayed = env.token_balance(&alice_shares) as f64 / USDC as f64 * env.ui_multiplier();
    println!("NAV after 3 days: {nav}; alice's wallet shows ${displayed:.6}");
    assert!(displayed > 1_000.4 && displayed < 1_000.42);

    let interest = 1_000 * USDC * (nav - NAV_SCALE) / NAV_SCALE;
    let custodian_topup = env.usdc_ata(&env.admin.pubkey());
    MintTo::new(
        &mut env.svm,
        &env.admin,
        &env.usdc_mint,
        &custodian_topup,
        interest,
    )
    .send()
    .unwrap();
    let t = spl_token::instruction::transfer_checked(
        &spl_token::id(),
        &custodian_topup,
        &env.usdc_mint,
        &env.vault,
        &env.admin.pubkey(),
        &[],
        interest,
        6,
    )
    .unwrap();
    let admin = env.admin.insecure_clone();
    env.send(t, &[&admin]).unwrap();

    env.redeem(&alice, 1_000 * USDC).unwrap();
    let alice_usdc = env.token_balance(&env.usdc_ata(&alice.pubkey()));
    println!(
        "alice redeemed for {} USDC (paid 1000)",
        alice_usdc as f64 / USDC as f64
    );
    assert_eq!(alice_usdc, 1_000 * USDC * nav / NAV_SCALE);
    assert!(alice_usdc > 1_000 * USDC);
    assert_eq!(env.token_balance(&alice_shares), 0);
}

#[test]
fn kyc_gates_subscribe_transfer_and_redeem() {
    let mut env = Env::new();
    let alice = env.new_investor(1_000 * USDC);
    let mallory = env.new_investor(1_000 * USDC);

    let err = env.subscribe(&alice, 100 * USDC).unwrap_err();
    assert!(err.contains("AccountNotInitialized"), "{err}");

    env.approve(&alice.pubkey());
    env.subscribe(&alice, 100 * USDC).unwrap();

    CreateAssociatedTokenAccount::new(&mut env.svm, &mallory, &env.share_mint)
        .owner(&mallory.pubkey())
        .token_program_id(&spl_token_2022::id())
        .send()
        .unwrap();
    let err = env
        .transfer_shares(&alice, &mallory.pubkey(), 10 * USDC)
        .unwrap_err();
    assert!(err.contains("Account is frozen"), "{err}");
    let err = env.subscribe(&mallory, 100 * USDC).unwrap_err();
    assert!(err.contains("Account is frozen"), "{err}");

    env.approve(&mallory.pubkey());
    env.transfer_shares(&alice, &mallory.pubkey(), 10 * USDC)
        .unwrap();

    let ix = env.set_investor_ix(&alice.pubkey(), false);
    let admin = env.admin.insecure_clone();
    env.send(ix, &[&admin]).unwrap();
    let err = env.redeem(&alice, 10 * USDC).unwrap_err();
    assert!(err.contains("Account is frozen"), "{err}");
    let err = env
        .transfer_shares(&alice, &mallory.pubkey(), 1)
        .unwrap_err();
    assert!(err.contains("Account is frozen"), "{err}");
}

#[test]
fn nav_guards_deviation_staleness_and_role() {
    let mut env = Env::new();
    let alice = env.new_investor(1_000 * USDC);
    env.approve(&alice.pubkey());

    let err = env.publish_nav(NAV_SCALE * 101 / 100).unwrap_err();
    assert!(err.contains("NavDeviationTooLarge"), "{err}");

    let ix = Instruction {
        program_id: tmmf::id(),
        accounts: tmmf::accounts::PublishNav {
            nav_manager: env.admin.pubkey(),
            nav_manager_role: role_pda(RoleType::NavManager, &env.admin.pubkey()),
            fund: env.fund,
            share_mint: env.share_mint,
            share_token_program: spl_token_2022::id(),
            event_authority: event_authority(),
            program: tmmf::id(),
        }
        .to_account_metas(None),
        data: tmmf::instruction::PublishNav {
            new_nav: NAV_SCALE + 1,
        }
        .data(),
    };
    let admin = env.admin.insecure_clone();
    let err = env.send(ix, &[&admin]).unwrap_err();
    assert!(err.contains("AccountNotInitialized"), "{err}");

    env.set_time(env.now() + 4 * DAY);
    let err = env.subscribe(&alice, 100 * USDC).unwrap_err();
    assert!(err.contains("StaleNav"), "{err}");
    env.publish_nav(accrue_one_day(NAV_SCALE)).unwrap();
    env.subscribe(&alice, 100 * USDC).unwrap();
}

#[test]
fn liquidity_pause_gate_and_sweep() {
    let mut env = Env::new();
    let alice = env.new_investor(10_000 * USDC);
    env.approve(&alice.pubkey());
    env.subscribe(&alice, 10_000 * USDC).unwrap();

    env.pause(false, true).unwrap();
    let err = env.redeem(&alice, USDC).unwrap_err();
    assert!(err.contains("RedemptionsPaused"), "{err}");
    env.resume(false, true).unwrap();

    env.redeem(&alice, 4_000 * USDC).unwrap();
    let err = env.redeem(&alice, 2_000 * USDC).unwrap_err();
    assert!(err.contains("RedemptionGateExceeded"), "{err}");
    env.set_time(env.now() + DAY);
    env.redeem(&alice, 2_000 * USDC).unwrap();

    let err = env.sweep(3_500 * USDC).unwrap_err();
    assert!(err.contains("BufferFloorBreached"), "{err}");
    env.sweep(3_000 * USDC).unwrap();
    assert_eq!(env.token_balance(&env.vault), 1_000 * USDC);
    let err = env.redeem(&alice, 2_000 * USDC).unwrap_err();
    assert!(err.contains("InsufficientLiquidity"), "{err}");
    env.redeem(&alice, 500 * USDC).unwrap();

    env.admin_ix(tmmf::instruction::SetMinBufferBps { min_buffer_bps: 0 }.data())
        .unwrap();
    let left = env.token_balance(&env.vault);
    env.sweep(left).unwrap();
    assert_eq!(env.token_balance(&env.vault), 0);
}

#[test]
fn init_survives_prefunded_share_mint() {
    let env = Env::new_with(true);
    let mint = env.svm.get_account(&env.share_mint).unwrap();
    assert_eq!(mint.owner, spl_token_2022::id());
    assert_eq!(env.fund().nav, NAV_SCALE);
}

#[test]
fn v1_fits_batch_kyc_onboarding_that_legacy_cannot() {
    let mut env = Env::new();
    let investors: Vec<Keypair> = (0..20).map(|_| Keypair::new()).collect();
    for i in &investors {
        CreateAssociatedTokenAccount::new(&mut env.svm, &env.admin, &env.share_mint)
            .owner(&i.pubkey())
            .token_program_id(&spl_token_2022::id())
            .send()
            .unwrap();
    }
    let ixs: Vec<Instruction> = investors
        .iter()
        .map(|i| env.set_investor_ix(&i.pubkey(), true))
        .collect();
    let admin = env.admin.insecure_clone();

    let legacy = env.tx(TxFormat::Legacy, &ixs, &[&admin]);
    let v1 = env.tx(TxFormat::V1, &ixs, &[&admin]);
    let (legacy_bytes, v1_bytes) = (wire_size(&legacy), wire_size(&v1));
    println!(
        "20 approvals, {} accounts: legacy {legacy_bytes} B (limit {LEGACY_MAX_TX_BYTES}), v1 {v1_bytes} B (limit {})",
        v1.message.static_account_keys().len(),
        v1::MAX_TRANSACTION_SIZE
    );
    assert!(
        legacy_bytes > LEGACY_MAX_TX_BYTES,
        "legacy would be dropped by the network"
    );
    assert!(v1_bytes <= v1::MAX_TRANSACTION_SIZE);

    env.send_tx(v1).unwrap();
    for i in &investors {
        let acc = env.svm.get_account(&env.shares_ata(&i.pubkey())).unwrap();
        let state = StateWithExtensions::<TokenAccount2022>::unpack(&acc.data).unwrap();
        assert!(!state.base.is_frozen(), "investor should be approved");
    }
}

fn budget(label: &str, used: u64, ceiling: u64) {
    println!("{label:<18}{used:>6} CU (budget {ceiling})");
    assert!(
        used <= ceiling,
        "{label} used {used} CU, over budget {ceiling}"
    );
}

#[test]
fn compute_unit_costs() {
    let mut env = Env::new_seeded(false, Some(7));
    budget("initialize_fund", env.init_cu, 80_000);
    budget("grant_role", env.last_cu, 20_000);
    let alice = env.new_investor(10_000 * USDC);
    env.approve(&alice.pubkey());
    budget("approve+create", env.last_cu, 95_000);
    let bob = env.new_investor_seeded(10 * USDC, Some(50));
    CreateAssociatedTokenAccount::new(&mut env.svm, &env.admin, &env.share_mint)
        .owner(&bob.pubkey())
        .token_program_id(&spl_token_2022::id())
        .send()
        .unwrap();
    env.approve(&bob.pubkey());
    budget("approve (thaw)", env.last_cu, 32_000);
    env.subscribe(&alice, 1_000 * USDC).unwrap();
    budget("subscribe", env.last_cu, 36_000);
    let now = env.now();
    env.post_usdc_price(100_000_000, -8, now);
    env.enable_usdc_guard(60, 100).unwrap();
    env.subscribe(&alice, 100 * USDC).unwrap();
    budget("subscribe+oracle", env.last_cu, 40_000);
    env.publish_nav(NAV_SCALE + 137_000).unwrap();
    budget("publish_nav", env.last_cu, 24_000);
    env.redeem(&alice, 100 * USDC).unwrap();
    budget("redeem", env.last_cu, 40_000);
    env.sweep(100 * USDC).unwrap();
    budget("sweep_to_custodian", env.last_cu, 26_000);
    env.resume(false, true).unwrap();
    budget("pause", env.last_cu, 10_000);
}

#[test]
fn only_the_upgrade_authority_can_launch_the_fund() {
    let mut svm = LiteSVM::new();
    let bytes = include_bytes!(concat!(env!("CARGO_TARGET_TMPDIR"), "/../deploy/tmmf.so"));
    svm.add_program(tmmf::id(), bytes).unwrap();
    let deployer = Keypair::new();
    let stranger = Keypair::new();
    set_upgrade_authority(&mut svm, Some(deployer.pubkey()));
    svm.airdrop(&stranger.pubkey(), 100_000_000_000).unwrap();

    let usdc_mint = CreateMint::new(&mut svm, &stranger)
        .decimals(6)
        .authority(&stranger.pubkey())
        .send()
        .unwrap();
    let custodian = CreateAssociatedTokenAccount::new(&mut svm, &stranger, &usdc_mint)
        .owner(&stranger.pubkey())
        .send()
        .unwrap();
    let (fund, _) = Pubkey::find_program_address(&[FUND_SEED], &tmmf::id());
    let (share_mint, _) =
        Pubkey::find_program_address(&[SHARE_MINT_SEED, fund.as_ref()], &tmmf::id());
    let ix = Instruction {
        program_id: tmmf::id(),
        accounts: tmmf::accounts::InitializeFund {
            admin: stranger.pubkey(),
            usdc_mint,
            fund,
            share_mint,
            vault: get_associated_token_address_with_program_id(
                &fund,
                &usdc_mint,
                &spl_token::id(),
            ),
            custodian,
            usdc_token_program: spl_token::id(),
            share_token_program: spl_token_2022::id(),
            associated_token_program: associated_token::ID,
            system_program: system_program::ID,
            program: tmmf::id(),
            program_data: get_program_data_address(&tmmf::id()),
        }
        .to_account_metas(None),
        data: tmmf::instruction::InitializeFund {
            params: FundParams {
                max_nav_age: 3 * DAY,
                max_nav_change_bps: 10,
                min_buffer_bps: 2_500,
                redemption_cap: 5_000 * USDC,
                redemption_window: DAY,
            },
        }
        .data(),
    };
    let blockhash = svm.latest_blockhash();
    let tx = build_tx(TxFormat::V1, &[ix], &[&stranger], blockhash);
    let err = svm.send_transaction(tx).unwrap_err();
    let logs = err.meta.pretty_logs();
    assert!(logs.contains("NotUpgradeAuthority"), "{logs}");
}

#[test]
fn roles_can_be_rotated_and_revoked() {
    let mut env = Env::new();
    let old_nav_manager = env.nav_manager.insecure_clone();
    let new_nav_manager = Keypair::new();
    env.svm
        .airdrop(&new_nav_manager.pubkey(), 1_000_000_000)
        .unwrap();

    let role = role_pda(RoleType::NavManager, &old_nav_manager.pubkey());
    assert!(env
        .svm
        .get_account(&role)
        .is_some_and(|a| !a.data.is_empty()));
    env.revoke_role(RoleType::NavManager, &old_nav_manager.pubkey())
        .unwrap();
    assert!(env.svm.get_account(&role).is_none_or(|a| a.data.is_empty()));

    env.set_time(env.now() + DAY);
    let err = env.publish_nav(NAV_SCALE + 100_000).unwrap_err();
    assert!(err.contains("AccountNotInitialized"), "{err}");

    env.grant_role(RoleType::NavManager, &new_nav_manager.pubkey())
        .unwrap();
    env.nav_manager = new_nav_manager;
    env.publish_nav(NAV_SCALE + 100_000).unwrap();
    assert_eq!(env.fund().nav, NAV_SCALE + 100_000);

    let mallory = Keypair::new();
    env.svm.airdrop(&mallory.pubkey(), 1_000_000_000).unwrap();
    let ix = Instruction {
        program_id: tmmf::id(),
        accounts: tmmf::accounts::GrantRole {
            payer: mallory.pubkey(),
            authority: mallory.pubkey(),
            role_to_grant: role_pda(RoleType::FundAdmin, &mallory.pubkey()),
            program: tmmf::id(),
            program_data: get_program_data_address(&tmmf::id()),
            system_program: system_program::ID,
        }
        .to_account_metas(None),
        data: tmmf::instruction::GrantRole {
            role: RoleType::FundAdmin,
            user: mallory.pubkey(),
        }
        .data(),
    };
    let err = env.send(ix, &[&mallory]).unwrap_err();
    assert!(err.contains("NotUpgradeAuthority"), "{err}");
}

#[test]
fn pauser_can_halt_but_not_resume() {
    let mut env = Env::new();
    let alice = env.new_investor(1_000 * USDC);
    env.approve(&alice.pubkey());

    env.pause(true, true).unwrap();
    let err = env.subscribe(&alice, 100 * USDC).unwrap_err();
    assert!(err.contains("SubscriptionsPaused"), "{err}");

    let admin = env.admin.pubkey();
    env.revoke_role(RoleType::FundAdmin, &admin).unwrap();
    let err = env.resume(true, true).unwrap_err();
    assert!(err.contains("AccountNotInitialized"), "{err}");
    env.pause(true, true).unwrap();

    env.grant_role(RoleType::FundAdmin, &admin).unwrap();
    env.resume(true, true).unwrap();
    env.subscribe(&alice, 100 * USDC).unwrap();
}

#[test]
fn usdc_depeg_guard_halts_dealing() {
    let mut env = Env::new();
    let alice = env.new_investor(10_000 * USDC);
    env.approve(&alice.pubkey());

    env.subscribe(&alice, 100 * USDC).unwrap();

    let now = env.now();
    env.post_usdc_price(100_000_000, -8, now);
    env.enable_usdc_guard(60, 100).unwrap();
    env.subscribe(&alice, 100 * USDC).unwrap();
    env.redeem(&alice, 50 * USDC).unwrap();

    env.post_usdc_price(99_500_000, -8, env.now());
    env.subscribe(&alice, 100 * USDC).unwrap();

    env.post_usdc_price(90_000_000, -8, env.now());
    let err = env.subscribe(&alice, 100 * USDC).unwrap_err();
    assert!(err.contains("UsdcDepegged"), "{err}");
    let err = env.redeem(&alice, 10 * USDC).unwrap_err();
    assert!(err.contains("UsdcDepegged"), "{err}");

    env.post_usdc_price(100_000_000, -8, env.now() - 600);
    let err = env.subscribe(&alice, 100 * USDC).unwrap_err();
    assert!(err.contains("OracleUnavailable"), "{err}");

    env.post_usdc_price(100_000_000, -8, env.now());
    let saved = env.usdc_price_update.take();
    let err = env.subscribe(&alice, 100 * USDC).unwrap_err();
    assert!(err.contains("OracleRequired"), "{err}");
    env.usdc_price_update = saved;

    env.admin_ix(
        tmmf::instruction::SetUsdcOracle {
            enabled: false,
            price_update: Pubkey::default(),
            feed_id: [0u8; 32],
            max_age: 0,
            max_deviation_bps: 0,
        }
        .data(),
    )
    .unwrap();
    env.post_usdc_price(90_000_000, -8, env.now());
    env.subscribe(&alice, 100 * USDC).unwrap();
}

#[test]
fn look_alike_program_data_is_rejected() {
    let mut env = Env::new();
    let mallory = Keypair::new();
    env.svm.airdrop(&mallory.pubkey(), 100_000_000_000).unwrap();

    let fake = Pubkey::new_from_array([42u8; 32]);
    let mut data = vec![0u8; 45];
    data[0] = 3;
    data[12] = 1;
    data[13..45].copy_from_slice(mallory.pubkey().as_ref());
    let lamports = env.svm.minimum_balance_for_rent_exemption(data.len());
    env.svm
        .set_account(
            fake,
            solana_account::Account {
                lamports,
                data,
                owner: anchor_lang::solana_program::bpf_loader_upgradeable::ID,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();

    let ix = Instruction {
        program_id: tmmf::id(),
        accounts: tmmf::accounts::GrantRole {
            payer: mallory.pubkey(),
            authority: mallory.pubkey(),
            role_to_grant: role_pda(RoleType::FundAdmin, &mallory.pubkey()),
            program: tmmf::id(),
            program_data: fake,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
        data: tmmf::instruction::GrantRole {
            role: RoleType::FundAdmin,
            user: mallory.pubkey(),
        }
        .data(),
    };
    let err = env.send(ix, &[&mallory]).unwrap_err();
    assert!(err.contains("NotUpgradeAuthority"), "{err}");
}

#[test]
fn custodian_and_nav_params_can_be_corrected() {
    let mut env = Env::new();
    let alice = env.new_investor(10_000 * USDC);
    env.approve(&alice.pubkey());
    env.subscribe(&alice, 9_000 * USDC).unwrap();

    let replacement = Keypair::new();
    env.svm
        .airdrop(&replacement.pubkey(), 1_000_000_000)
        .unwrap();
    let new_custodian = CreateAssociatedTokenAccount::new(&mut env.svm, &env.admin, &env.usdc_mint)
        .owner(&replacement.pubkey())
        .send()
        .unwrap();

    let admin = env.admin.insecure_clone();
    let ix = Instruction {
        program_id: tmmf::id(),
        accounts: tmmf::accounts::SetCustodian {
            admin: admin.pubkey(),
            admin_role: role_pda(RoleType::FundAdmin, &admin.pubkey()),
            fund: env.fund,
            usdc_mint: env.usdc_mint,
            custodian: new_custodian,
            usdc_token_program: spl_token::id(),
        }
        .to_account_metas(None),
        data: tmmf::instruction::SetCustodian {}.data(),
    };
    env.send(ix, &[&admin]).unwrap();
    assert_eq!(env.fund().custodian, new_custodian);

    env.custodian = new_custodian;
    env.sweep(1_000 * USDC).unwrap();
    assert_eq!(env.token_balance(&new_custodian), 1_000 * USDC);

    env.set_time(env.now() + 10 * DAY);
    let err = env.subscribe(&alice, USDC).unwrap_err();
    assert!(err.contains("StaleNav"), "{err}");

    env.admin_ix(
        tmmf::instruction::SetNavParams {
            max_nav_age: 30 * DAY,
            max_nav_change_bps: 50,
        }
        .data(),
    )
    .unwrap();
    env.subscribe(&alice, USDC).unwrap();
    env.publish_nav(NAV_SCALE + 4_000_000).unwrap();
}

#[test]
fn config_guards_reject_settings_that_would_brick_the_fund() {
    let mut env = Env::new();

    let err = env
        .admin_ix(
            tmmf::instruction::SetRedemptionGate {
                cap: 0,
                window_seconds: DAY,
            }
            .data(),
        )
        .unwrap_err();
    assert!(err.contains("InvalidParams"), "{err}");

    env.post_usdc_price(100_000_000, -8, env.now());
    let err = env.enable_usdc_guard(60, 0).unwrap_err();
    assert!(err.contains("InvalidParams"), "{err}");

    let err = env
        .admin_ix(
            tmmf::instruction::SetNavParams {
                max_nav_age: 0,
                max_nav_change_bps: 50,
            }
            .data(),
        )
        .unwrap_err();
    assert!(err.contains("InvalidParams"), "{err}");

    let err = env
        .admin_ix(
            tmmf::instruction::SetNavParams {
                max_nav_age: DAY,
                max_nav_change_bps: 10_000,
            }
            .data(),
        )
        .unwrap_err();
    assert!(err.contains("InvalidParams"), "{err}");
}
