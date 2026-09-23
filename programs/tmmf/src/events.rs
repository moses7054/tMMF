use anchor_lang::prelude::*;

use crate::RoleType;

#[event]
pub struct InvestorStatusChanged {
    pub fund: Pubkey,
    pub investor: Pubkey,
    pub approved: bool,
}

#[event]
pub struct Subscribed {
    pub fund: Pubkey,
    pub investor: Pubkey,
    pub usdc_in: u64,
    pub shares_out: u64,
    pub nav: u64,
}

#[event]
pub struct Redeemed {
    pub fund: Pubkey,
    pub investor: Pubkey,
    pub shares_in: u64,
    pub usdc_out: u64,
    pub nav: u64,
}

#[event]
pub struct NavPublished {
    pub fund: Pubkey,
    pub old_nav: u64,
    pub new_nav: u64,
    pub timestamp: i64,
}

#[event]
pub struct RoleGranted {
    pub role: RoleType,
    pub grantee: Pubkey,
    pub granter: Pubkey,
}

#[event]
pub struct RoleRevoked {
    pub role: RoleType,
    pub grantee: Pubkey,
    pub revoker: Pubkey,
}
