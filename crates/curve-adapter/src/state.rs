use crate::CurveVariant;

/// How often a state field needs to be updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateFrequency {
    /// Changes every swap/deposit/withdraw. Must be tracked per-block.
    PerBlock,
    /// Changes on admin events (A ramping, fee changes). Days between updates.
    SemiStatic,
    /// Set at pool creation, never changes.
    Static,
}

// # Reading the amplification parameter (A / amp)
//
// ## The `A()` getter is lossy — do NOT use it
//
// Curve pools store amplification as `initial_A` (raw value, includes A_PRECISION multiplier).
// The public `A()` getter returns `initial_A / A_PRECISION` via integer division.
// When `initial_A % A_PRECISION != 0`, the remainder is lost:
//
// ```text
// initial_A = 79258
// A() = 79258 / 100 = 792  (integer division)
// A() * 100 = 79200 ≠ 79258  (lost 58)
// ```
//
// The on-chain `get_D` and `get_y` use `_A()` internally which returns the raw `initial_A`
// value (or interpolated value during ramping). Using `A() * A_PRECISION` introduces a
// small error that causes wei-level mismatches in swap calculations.
//
// ## Correct approach: read `initial_A` and `future_A` directly
//
// All StableSwap V2+ pools expose:
// - `initial_A() -> uint256` — raw start value (includes A_PRECISION)
// - `future_A() -> uint256` — raw end value (includes A_PRECISION)
// - `initial_A_time() -> uint256` — ramp start timestamp
// - `future_A_time() -> uint256` — ramp end timestamp
//
// V0/V1 pools do NOT have these getters — use `A()` directly (A_PRECISION=1).
//
// CryptoSwap pools expose `initial_A_gamma()` and `future_A_gamma()` which return
// packed values containing both A and gamma. The A in these packed values already
// includes A_MULTIPLIER — use directly without further scaling.
//
// ## Ramping interpolation
//
// During A ramping (which can last days), the consumer must interpolate:
//
// ```text
// if now < future_A_time:
//     amp = initial_A + (future_A - initial_A) * (now - initial_A_time) / (future_A_time - initial_A_time)
// else:
//     amp = future_A
// ```
//
// This is the same formula Curve uses internally in `_A()`.
// The indexer provides all four values; the consumer interpolates at swap time
// using the current block timestamp.
//
// ## For factory deploy events
//
// Factory `PlainPoolDeployed` and `MetaPoolDeployed` events emit `A` as the user-provided
// value (e.g. 400), not the raw on-chain value (e.g. 40000). When converting event values
// to the format expected by curve-math, multiply by A_PRECISION (100 for V2/NG/Meta/ALend).
//
// # Reading balances
//
// ## StableSwapNG: `balances()` ≠ ERC20 `balanceOf(pool)`
//
// StableSwapNG pools compute `balances(i) = ERC20.balanceOf(pool) - admin_balances[i]`.
// The `balanceOf` includes uncollected admin fees; `balances()` excludes them.
// The on-chain swap math uses `balances()` (without admin fees).
// An indexer tracking ERC20 Transfer events sees `balanceOf` changes, not `balances()`.
// The difference is small (admin fees) but causes wei-level mismatches.
//
// ## Rebase tokens (e.g. stETH)
//
// Pools containing rebase tokens (stETH, etc.) have balances that change without
// Transfer events — the token rebases in-place. These pools require an entrypoint
// that calls `balanceOf(pool)` on the rebase token to track the actual balance.
// The stETH/ETH pool additionally tracks native ETH via `self.balance` (contract
// ETH balance), not via any ERC20 mechanism.
//
// # MetaPool base pool detection
//
// The old MetaPool Factory (`0xB9fC...`) deploys both plain and meta pools via the
// same contract. The factory exposes `is_meta(pool) -> bool` to distinguish them.
// Alternatively, listen for different deploy events:
// - `PlainPoolDeployed` → plain pool (StableSwapV2)
// - `MetaPoolDeployed` → meta pool (StableSwapMeta)
//
// Proxy pools from this factory lack a `base_pool()` getter. Use the factory's
// `get_base_pool(pool) -> address` instead if RPC is available.

/// A named state field with its update frequency.
#[derive(Debug, Clone)]
pub struct StateField {
    /// Field name as used in curve-math's Pool enum.
    pub name: &'static str,
    /// How often this field changes.
    pub frequency: UpdateFrequency,
    /// On-chain function to read this field (for RPC-based indexers).
    pub on_chain_getter: &'static str,
}

/// Complete state requirements for a pool variant.
#[derive(Debug, Clone)]
pub struct StateRequirements {
    /// All fields needed to construct a curve-math Pool for this variant.
    pub fields: Vec<StateField>,
}

/// Get state requirements for a given variant.
///
/// Returns the complete list of on-chain fields needed to construct
/// a `curve_math::Pool` enum for this variant, along with how often
/// each field changes.
pub fn state_requirements(variant: CurveVariant) -> StateRequirements {
    match variant {
        CurveVariant::StableSwapV0 => StateRequirements {
            fields: vec![
                StateField { name: "balances", frequency: UpdateFrequency::PerBlock, on_chain_getter: "balances(int128)" },
                StateField { name: "rates", frequency: UpdateFrequency::Static, on_chain_getter: "— computed from token decimals: 10^(36-decimals)" },
                StateField { name: "amp", frequency: UpdateFrequency::Static, on_chain_getter: "A() — V0 has no ramping, A_PRECISION=1" },
                StateField { name: "fee", frequency: UpdateFrequency::Static, on_chain_getter: "fee()" },
            ],
        },

        CurveVariant::StableSwapV1 => StateRequirements {
            fields: vec![
                StateField { name: "balances", frequency: UpdateFrequency::PerBlock, on_chain_getter: "balances(uint256)" },
                StateField { name: "rates", frequency: UpdateFrequency::Static, on_chain_getter: "— computed from token decimals: 10^(36-decimals)" },
                StateField { name: "amp", frequency: UpdateFrequency::Static, on_chain_getter: "A() — V1 has no ramping, A_PRECISION=1" },
                StateField { name: "fee", frequency: UpdateFrequency::Static, on_chain_getter: "fee()" },
            ],
        },

        CurveVariant::StableSwapV2 => StateRequirements {
            fields: vec![
                StateField { name: "balances", frequency: UpdateFrequency::PerBlock, on_chain_getter: "balances(uint256)" },
                StateField { name: "rates", frequency: UpdateFrequency::Static, on_chain_getter: "— computed from token decimals: 10^(36-decimals)" },
                StateField { name: "amp", frequency: UpdateFrequency::SemiStatic, on_chain_getter: "initial_A() + future_A() + timestamps — see module docs. Do NOT use A()." },
                StateField { name: "fee", frequency: UpdateFrequency::Static, on_chain_getter: "fee()" },
            ],
        },

        // Meta pools: rates[0] is static (from decimals), but rates[1] = virtual_price
        // from the base pool, which changes every block.
        CurveVariant::StableSwapMeta => StateRequirements {
            fields: vec![
                StateField { name: "balances", frequency: UpdateFrequency::PerBlock, on_chain_getter: "balances(uint256)" },
                StateField { name: "rates", frequency: UpdateFrequency::PerBlock, on_chain_getter: "rates[0] = 10^(36-decimals) (static); rates[1] = base_pool.get_virtual_price() (per-block)" },
                StateField { name: "amp", frequency: UpdateFrequency::SemiStatic, on_chain_getter: "initial_A() + future_A() + timestamps — see module docs. Do NOT use A()." },
                StateField { name: "fee", frequency: UpdateFrequency::Static, on_chain_getter: "fee()" },
            ],
        },

        CurveVariant::StableSwapALend => StateRequirements {
            fields: vec![
                StateField { name: "balances", frequency: UpdateFrequency::PerBlock, on_chain_getter: "balances(uint256)" },
                StateField { name: "precision_mul", frequency: UpdateFrequency::Static, on_chain_getter: "— computed from token decimals" },
                StateField { name: "amp", frequency: UpdateFrequency::SemiStatic, on_chain_getter: "initial_A() + future_A() + timestamps — see module docs. Do NOT use A()." },
                StateField { name: "fee", frequency: UpdateFrequency::Static, on_chain_getter: "fee()" },
                StateField { name: "offpeg_fee_multiplier", frequency: UpdateFrequency::Static, on_chain_getter: "offpeg_fee_multiplier()" },
            ],
        },

        CurveVariant::StableSwapNG => StateRequirements {
            fields: vec![
                StateField { name: "balances", frequency: UpdateFrequency::PerBlock, on_chain_getter: "balances(uint256)" },
                StateField { name: "rates", frequency: UpdateFrequency::PerBlock, on_chain_getter: "stored_rates() — dynamic for oracle tokens, static for plain" },
                StateField { name: "amp", frequency: UpdateFrequency::SemiStatic, on_chain_getter: "initial_A() + future_A() + timestamps — see module docs. Do NOT use A()." },
                StateField { name: "fee", frequency: UpdateFrequency::Static, on_chain_getter: "fee()" },
                StateField { name: "offpeg_fee_multiplier", frequency: UpdateFrequency::Static, on_chain_getter: "offpeg_fee_multiplier()" },
            ],
        },

        CurveVariant::TwoCryptoV1
        | CurveVariant::TwoCryptoNG => StateRequirements {
            fields: vec![
                StateField { name: "balances", frequency: UpdateFrequency::PerBlock, on_chain_getter: "balances(uint256)" },
                StateField { name: "precisions", frequency: UpdateFrequency::Static, on_chain_getter: "precisions()" },
                StateField { name: "price_scale", frequency: UpdateFrequency::PerBlock, on_chain_getter: "price_scale()" },
                StateField { name: "d", frequency: UpdateFrequency::PerBlock, on_chain_getter: "D()" },
                StateField { name: "ann", frequency: UpdateFrequency::SemiStatic, on_chain_getter: "A()" },
                StateField { name: "gamma", frequency: UpdateFrequency::SemiStatic, on_chain_getter: "gamma()" },
                StateField { name: "mid_fee", frequency: UpdateFrequency::Static, on_chain_getter: "mid_fee()" },
                StateField { name: "out_fee", frequency: UpdateFrequency::Static, on_chain_getter: "out_fee()" },
                StateField { name: "fee_gamma", frequency: UpdateFrequency::Static, on_chain_getter: "fee_gamma()" },
            ],
        },

        CurveVariant::TwoCryptoStable => StateRequirements {
            fields: vec![
                StateField { name: "balances", frequency: UpdateFrequency::PerBlock, on_chain_getter: "balances(uint256)" },
                StateField { name: "precisions", frequency: UpdateFrequency::Static, on_chain_getter: "precisions()" },
                StateField { name: "price_scale", frequency: UpdateFrequency::PerBlock, on_chain_getter: "price_scale()" },
                StateField { name: "d", frequency: UpdateFrequency::PerBlock, on_chain_getter: "D()" },
                StateField { name: "ann", frequency: UpdateFrequency::SemiStatic, on_chain_getter: "A()" },
                // no gamma field — TwoCryptoStable ignores gamma
                StateField { name: "mid_fee", frequency: UpdateFrequency::Static, on_chain_getter: "mid_fee()" },
                StateField { name: "out_fee", frequency: UpdateFrequency::Static, on_chain_getter: "out_fee()" },
                StateField { name: "fee_gamma", frequency: UpdateFrequency::Static, on_chain_getter: "fee_gamma()" },
            ],
        },

        CurveVariant::TriCryptoV1
        | CurveVariant::TriCryptoNG => StateRequirements {
            fields: vec![
                StateField { name: "balances", frequency: UpdateFrequency::PerBlock, on_chain_getter: "balances(uint256)" },
                StateField { name: "precisions", frequency: UpdateFrequency::Static, on_chain_getter: "precisions()" },
                StateField { name: "price_scale", frequency: UpdateFrequency::PerBlock, on_chain_getter: "price_scale(uint256) — returns 2 values (index 0 and 1)" },
                StateField { name: "d", frequency: UpdateFrequency::PerBlock, on_chain_getter: "D()" },
                StateField { name: "ann", frequency: UpdateFrequency::SemiStatic, on_chain_getter: "A()" },
                StateField { name: "gamma", frequency: UpdateFrequency::SemiStatic, on_chain_getter: "gamma()" },
                StateField { name: "mid_fee", frequency: UpdateFrequency::Static, on_chain_getter: "mid_fee()" },
                StateField { name: "out_fee", frequency: UpdateFrequency::Static, on_chain_getter: "out_fee()" },
                StateField { name: "fee_gamma", frequency: UpdateFrequency::Static, on_chain_getter: "fee_gamma()" },
            ],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stableswap_needs_4_fields() {
        let reqs = state_requirements(CurveVariant::StableSwapV1);
        assert_eq!(reqs.fields.len(), 4);
    }

    #[test]
    fn twocrypto_stable_has_no_gamma() {
        let reqs = state_requirements(CurveVariant::TwoCryptoStable);
        assert!(!reqs.fields.iter().any(|f| f.name == "gamma"));
    }

    #[test]
    fn tricrypto_has_9_fields() {
        let reqs = state_requirements(CurveVariant::TriCryptoNG);
        assert_eq!(reqs.fields.len(), 9);
    }
}
