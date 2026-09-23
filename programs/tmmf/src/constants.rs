use anchor_lang::prelude::*;

#[constant]
pub const FUND_SEED: &[u8] = b"fund";

#[constant]
pub const SHARE_MINT_SEED: &[u8] = b"shares";

#[constant]
pub const NAV_SCALE: u64 = 1_000_000_000;

#[constant]
pub const BPS_DENOMINATOR: u64 = 10_000;
