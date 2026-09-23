use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Only the program upgrade authority can launch the fund")]
    NotUpgradeAuthority,
    #[msg("Invalid fund parameters")]
    InvalidParams,
    #[msg("Subscriptions are paused")]
    SubscriptionsPaused,
    #[msg("Redemptions are paused")]
    RedemptionsPaused,
    #[msg("NAV is stale; waiting for the NAV manager to publish")]
    StaleNav,
    #[msg("NAV change exceeds the allowed deviation")]
    NavDeviationTooLarge,
    #[msg("Redemption would exceed the gate for this window")]
    RedemptionGateExceeded,
    #[msg("Not enough USDC in the vault; the fund must recall cash from the custodian")]
    InsufficientLiquidity,
    #[msg("Sweep would leave the vault below the minimum cash buffer")]
    BufferFloorBreached,
    #[msg("USDC price oracle account is required while the depeg guard is on")]
    OracleRequired,
    #[msg("USDC price is unavailable, stale or unverified")]
    OracleUnavailable,
    #[msg("Oracle reported a non-positive price")]
    InvalidOraclePrice,
    #[msg("USDC has moved too far from $1.00; dealing is halted")]
    UsdcDepegged,
    #[msg("Math overflow")]
    MathOverflow,
}
