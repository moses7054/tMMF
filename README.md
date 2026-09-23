# tmmf

A tokenized money market fund on Solana. Investors pay USDC and receive shares priced at NAV. 
The usdc obtained is swept to a custodian that buys Treasure bills off-chain. The daily interest on the Tresure bills held raises the NAV therfore making the shares more valuable.Shares are a Token-2022 mint tokens that only KYC-approved wallets can hold.


Funds of this shape include Franklin Templeton's BENJI, BlackRock's BUIDL, and Ondo's USDY.

## How the fund works

```
                  approve_investor (KYC: unfreeze the share account)
                            |
  Investor  --- subscribe: USDC ------> vault (fund PDA)
            <-- shares minted --------- share mint          |
                                                            | sweep_to_custodian
                                                            v
                                             custodian buys T-bills (off-chain)
                                                            |
  publish_nav (daily) <--- NAV manager: "a share is now worth 1.000137"
            |                                               |
  wallet balances grow                interest returns to the vault as USDC
            |                                               |
  Investor  --- redeem: shares burned --> vault pays USDC at the new NAV
```

The fund grows in two ways. Subscriptions add cash and shares together, so NAV does not move and nobody's holding changes value. Interest adds assets while the share count stays fixed, so NAV rises and every existing share is worth more. Interest Yield do not mints new shares.

## Accounts

### Fund

One per program, at seeds `["fund"]`. Holding it in a singleton PDA means nobody can deploy a second fund beside this one and pass it off as the real thing.

| Field | Purpose |
| --- | --- |
| `share_mint`, `usdc_mint`, `vault`, `custodian` | The four addresses used for validation |
| `decimals` | Shares copy USDC's decimals, so one cached value serves both sides of a trade |
| `nav` | Price of one share in USDC, scaled by 1e9. `1_000_000_000` is $1.00 |
| `nav_updated_at`, `max_nav_age` | Dealing stops when the NAV gets stale |
| `max_nav_change_bps` | Largest NAV move a single publish may make |
| `subscriptions_paused`, `redemptions_paused` | Set by a pauser, cleared by an admin |
| `oracle_enabled`, `usdc_price_update`, `usdc_feed_id`, `usdc_max_age`, `usdc_max_deviation_bps` | The USDC depeg guard |
| `min_buffer_bps` | Minimum Cash have to be maintained in the Vault |
| `redemption_cap`, `redemption_window`, `window_start`, `redeemed_in_window` | The redemption gate |
| `bump` | Stored so later instructions verify the PDA with one hash instead of a search |

### Role

Each granted permission gets its own account, at seeds [role.seed(), user]. Creating it grants the role, closing it takes the role away and refunds the rent.

| Role | What it can do |
| --- | --- |
| `FundAdmin` | KYC approvals and revocations, resume dealing, configure the gate, buffer and oracle, sweep to the custodian |
| `NavManager` | Publish NAV. Nothing else |
| `Pauser` | Halt dealing, but not restart it |

Only the program's upgrade authority can grant or revoke a role. In production that key should be a multisig.

### Share mint

A Token-2022 mint at seeds `["shares", fund]`, with the fund PDA as both mint authority and freeze authority. Two extensions carry most of the design:

`ScaledUiAmount` sets the UI multiplier to the NAV, so a wallet shows the dollar value of a holding. An investor watches a $1,000 balance grow to $1,000.41 while their raw share count stays at 1,000. BENJI does the same thing by changing share counts; USYC does it by price. This gets both readings out of one mechanism.

`DefaultAccountState = Frozen` makes every new share account arrive frozen. Approval thaws it. Because the freeze lives on the token account, the token program enforces KYC on ordinary transfers too, not only on instructions this program handles. A whitelist checked inside the program would leave wallet-to-wallet transfers open.

## Instructions

### initialize_fund

Creates the fund state, the share mint and the USDC vault, with NAV at 1.00 and the depeg guard off. The custodian is passed as a token account and checked against the USDC mint, so a wrong address cannot be stored.

The signer must be the program's upgrade authority. The `ProgramData` constraint proves the signer is an upgrade authority, and `require_own_program_data` proves the `ProgramData` account passed in belongs to this program rather than another one that happens to name the same signer. Without the second check, anyone could point at a program they control and launch the fund.

The share mint is built by hand because both extensions must be initialized before `InitializeMint`, which Anchor's `init` cannot express. The account is created with the same fund, allocate and assign sequence Anchor generates, so a stranger cannot block the launch by sending a few lamports to the mint's derivable address first.

### approve_investor

Thaws an investor's share account. `FundAdmin` only.

`init_if_needed` will create the account for an investor who has not opened one, which costs about 21,000 compute units and 7 entries of the runtime's 64-entry instruction trace. When investors create their own account first, approval is a thaw and an event: 2 trace entries, so 28 approvals fit in one transaction instead of 9.

Approving an already-approved investor does nothing, because thawing a thawed account is an error in the token program.

### revoke_investor

Freezes an investor's share account. `FundAdmin` only. Their shares still exist, but cannot move, be redeemed, or be added to.

This instruction never creates anything. An investor with no share account has nothing to freeze, so it takes neither the associated token program nor the system program.

### subscribe

Buys shares with USDC at the current NAV. Checks, in order:

1. Subscriptions are not paused.
2. The amount is above zero.
3. The NAV is not stale.
4. USDC is near a dollar, if the guard is on.
5. `shares = usdc * 1e9 / nav`, rounded down, is above zero.

Then USDC moves to the vault and the fund mints shares. A frozen share account makes the mint fail, so KYC is enforced by the token program rather than by a check here.

Rounding down leaves any dust with the fund, which protects existing holders.

### redeem

Burns shares for USDC at the current NAV. Same first four checks as `subscribe`, then:

1. `usdc = shares * nav / 1e9`, rounded down, is above zero.
2. The vault holds that much cash, or the call fails with `InsufficientLiquidity`.
3. The redemption gate has capacity left in this window.

Shares burn before the vault pays. If the payout fails the whole transaction reverts, so an investor cannot lose shares without being paid.

A redemption may draw the vault below the buffer floor. That is what the buffer is for. Only the admin is blocked from sweeping it away.

### publish_nav

Records a new NAV and updates the mint's UI multiplier. `NavManager` only.

The fund administrator computes NAV off-chain from the custodian's holdings, accruals and fees. No market oracle can derive it, which is why this is attested publication rather than price discovery. RedStone's Trusted Single Source Oracle and Chainlink's NAVLink solve the same problem with a signed feed instead of a role key.

A move larger than `max_nav_change_bps` is rejected. Real money market NAVs change by fractions of a basis point per day, so anything larger is a typo or a stolen key. Publishing also refreshes `nav_updated_at`, which is what keeps dealing open.

### sweep_to_custodian

Moves idle cash from the vault to the custodian. `FundAdmin` only.

The custodian address is fixed at launch and pinned by `has_one`, so the admin can send cash there and nowhere else. After the sweep the vault must still hold `min_buffer_bps` of assets, where assets are `share_supply * nav`. Money market funds carry a daily liquidity floor, and this is the on-chain form of it.

Cash comes back from the custodian as a plain USDC transfer into the vault, so there is no matching instruction.

### pause and resume

`pause` requires the `Pauser` role and can only set the flags. `resume` requires `FundAdmin` and can only clear them. The split is deliberate: halting a fund during a market disruption should be available to more keys than restarting it.

### set_redemption_gate, set_min_buffer_bps, set_nav_params, set_usdc_oracle

`FundAdmin` settings that share one accounts struct. The gate caps USDC redeemed per window. The buffer sets the cash floor. `set_nav_params` changes the staleness limit and the per-publish deviation cap. The oracle setting points the fund at a Pyth USDC/USD feed, and can switch the guard off, because a fund that cannot disable a broken feed cannot trade.

Each one rejects a value that would leave the fund unusable: a redemption cap of zero, a NAV age of zero, a deviation cap of 10,000 bps, and a zero price tolerance while the oracle guard is on. A zero tolerance would halt dealing unless USDC printed at exactly one dollar.

### set_custodian

Points the fund at a different custodian account. `FundAdmin` only.

The account is passed and validated rather than taken as a raw pubkey, so it has to be a USDC token account before it can be stored. `initialize_fund` does the same. Without this instruction a mistyped custodian at launch would strand the fund's cash, because the fund PDA is a singleton and cannot be recreated.

### grant_role and revoke_role

Create or close a `Role` PDA. Upgrade authority only, with the same two-part `ProgramData` check as `initialize_fund`. Revoking closes the account and returns the rent to a recipient.

Rotating a compromised NAV manager is: revoke the old role, grant the same role to the new key. Fund state is untouched, and no redeploy is needed.

## The USDC depeg guard

The fund prices USDC at a dollar. If USDC trades at 90 cents and the guard is off, an investor can subscribe with impaired dollars and get shares valued at par.

With the guard on, the client posts a Pyth price update in the same transaction. `get_price_no_older_than` rejects an update that is stale, partially verified, or for the wrong feed, then `require_usdc_at_par` rescales `price * 10^exponent` and rejects anything beyond the configured tolerance. Ondo does the same thing in its USDC swap paths.

The feed id is stored in `Fund` rather than hardcoded, because feed ids and receiver accounts differ between clusters.

## Why some accounts are unchecked

`subscribe` and `redeem` declare `share_mint` and `usdc_mint` as `UncheckedAccount`. Both addresses are pinned by `has_one` on the fund, and the token program validates the mints during the CPI. Deserializing a Token-2022 mint with extensions costs compute units and buys nothing here, since the only field these paths need is `decimals`, which is cached on `Fund` and re-verified by every `*_checked` CPI.

`initialize_fund` declares `share_mint` unchecked because the account does not exist yet. It is created inside the handler.

## Building and testing

```bash
anchor build
cargo test -p tmmf --test mod -- --nocapture
```

Tests run in LiteSVM and send every transaction in the v1 format. Anchor 1.2, LiteSVM 0.16.

The suite covers the full lifecycle (subscribe, three days of accrual, redeem for more than was paid), KYC on transfers and redemptions, NAV guards, the pause and gate and buffer, role rotation, the depeg guard, and two regressions: a pre-funded share mint address, and a stranger trying to launch the fund.

`compute_unit_costs` prints and asserts a budget per instruction:

| Instruction | CU |
| --- | --- |
| `initialize_fund` | ~46,000 |
| `subscribe` | ~19,800, or ~20,900 with the depeg guard on |
| `redeem` | ~22,600 |
| `approve_investor`, thaw only | ~14,000 |
| `approve_investor`, creating the account | 33,000 to 48,000 |
| `publish_nav` | ~15,000 |
| `sweep_to_custodian` | ~14,500 |
| `grant_role` | ~10,000 |
| `pause` | ~6,900 |


## Transaction limits

Onboarding 20 investors in one transaction needs 47 accounts and 1,846 bytes. That exceeds the legacy limit of 1,232 bytes, and fits the v1 limit of 4,096. v1 does not raise the other two ceilings: 64 accounts and 64 trace entries per transaction stay where they were.

So the batch size depends on which ceiling binds first. Approvals that create accounts hit the trace limit at 9. Approvals that only thaw hit the account limit at 28.

