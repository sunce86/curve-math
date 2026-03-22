//! Unified `Pool` enum covering all Curve pool variants.
//!
//! # Variant identification
//!
//! | Variant          | Era / Template                    | Key traits                                   | Example pools                        |
//! |------------------|-----------------------------------|----------------------------------------------|--------------------------------------|
//! | `StableSwapV0`   | Earliest (2020)                   | A_PRECISION=1, no -1, fee after denorm       | sUSD, Compound, USDT, y, BUSD        |
//! | `StableSwapV1`   | 3pool era (2021)                  | A_PRECISION=1, -1 offset, fee after denorm   | 3pool, ren, sbtc, hbtc               |
//! | `StableSwapV2`   | Base/plain template               | A_PRECISION=100, -1, fee before denorm       | FRAX/USDC, stETH, factory plain      |
//! | `StableSwapALend`| Aave lending template             | A_PRECISION=100, no -1, dynamic fee, PREC_MUL| Aave, sAAVE, IB, aETH               |
//! | `StableSwapNG`   | StableSwap-NG factory             | A_PRECISION=100, -1, dynamic fee, before dnrm| NG plain + meta pools                |
//! | `StableSwapMeta` | Meta pool template                | A_PRECISION=100, -1, fee before denorm       | GUSD, HUSD, factory meta             |
//! | `TwoCryptoV1`    | Legacy CurveCryptoSwap2           | 2-coin, Newton y, price_scale                | CRV/ETH                              |
//! | `TwoCryptoNG`    | twocrypto-ng factory              | 2-coin, Cardano cubic y, price_scale         | crvUSD/FXN, most new 2-coin crypto   |
//! | `TriCryptoV1`    | Legacy tricrypto                  | 3-coin, Newton y, 2 price_scales             | tricrypto2 (USDT/WBTC/WETH)          |
//! | `TriCryptoNG`    | tricrypto-ng factory              | 3-coin, hybrid cubic y, 2 price_scales       | tricrypto-ng (USDC/WBTC/WETH)        |
//!
//! **How to identify which variant to use:**
//! - Pools from a known factory → the factory determines the variant (NG, twocrypto-ng, tricrypto-ng)
//! - Legacy pools → match by pool address against known deployments
//! - If the pool has `gamma()` → CryptoSwap; otherwise StableSwap
//! - If StableSwap has `offpeg_fee_multiplier()` → ALend or NG
//! - Check `A_PRECISION` (1 for V0/V1, 100 for V2/Meta/ALend/NG)

use alloy_primitives::U256;

use crate::swap;

/// All known Curve pool variants with their parameters.
///
/// # State update frequency for indexer/adapter implementations
///
/// Each field falls into one of three categories:
///
/// | Category | When to update | Fields |
/// |----------|---------------|--------|
/// | **Per-block** | Every block (or on every swap event) | `balances`, `rates` (if oracle), `d`, `price_scale` |
/// | **Per-event** | On admin events (rare, days/weeks) | `amp` (during A ramping) |
/// | **Static** | Once at pool creation | `fee`, `offpeg_fee_multiplier`, `precision_mul` |
///
/// For `rates`: plain tokens have static rates (`10^(36-decimals)`), but ERC4626/oracle
/// tokens have dynamic rates that change per-block. Read `stored_rates()` from the pool
/// contract to get current rates for oracle-enabled pools.
#[derive(Clone)]
pub enum Pool {
    /// Oldest StableSwap pools (sUSD, Compound, USDT, y, BUSD).
    /// A_PRECISION=1, no -1 offset, fee after denormalize.
    StableSwapV0 {
        balances: Vec<U256>,
        rates: Vec<U256>,
        amp: U256,
        fee: U256,
    },

    /// 3pool-era StableSwap (3pool, ren, sbtc, hbtc).
    /// A_PRECISION=1, -1 offset, fee after denormalize.
    StableSwapV1 {
        balances: Vec<U256>,
        rates: Vec<U256>,
        amp: U256,
        fee: U256,
    },

    /// Base/plain template StableSwap (FRAX/USDC, stETH, factory plain).
    /// A_PRECISION=100, -1 offset, fee before denormalize.
    StableSwapV2 {
        balances: Vec<U256>,
        rates: Vec<U256>,
        amp: U256,
        fee: U256,
    },

    /// Aave-style lending StableSwap (Aave, sAAVE, IB, aETH).
    /// A_PRECISION=100, no -1 offset, dynamic fee, uses precision_mul.
    StableSwapALend {
        balances: Vec<U256>,
        precision_mul: Vec<U256>,
        amp: U256,
        fee: U256,
        offpeg_fee_multiplier: U256,
    },

    /// StableSwap-NG (plain + meta NG pools).
    /// A_PRECISION=100, -1 offset, dynamic fee, fee before denormalize.
    StableSwapNG {
        balances: Vec<U256>,
        rates: Vec<U256>,
        amp: U256,
        fee: U256,
        offpeg_fee_multiplier: U256,
    },

    /// Meta pool StableSwap (GUSD/3CRV, HUSD, factory meta).
    /// A_PRECISION=100, -1 offset, fee before denormalize.
    /// `rates[1]` must be set to fresh `virtual_price` from base pool.
    StableSwapMeta {
        balances: Vec<U256>,
        rates: Vec<U256>,
        amp: U256,
        fee: U256,
    },

    /// Legacy 2-coin CryptoSwap (CurveCryptoSwap2).
    /// Newton iteration for y.
    TwoCryptoV1 {
        balances: [U256; 2],
        precisions: [U256; 2],
        price_scale: U256,
        d: U256,
        ann: U256,
        gamma: U256,
        mid_fee: U256,
        out_fee: U256,
        fee_gamma: U256,
    },

    /// Next-gen 2-coin CryptoSwap (twocrypto-ng).
    /// Cardano cubic formula for y.
    TwoCryptoNG {
        balances: [U256; 2],
        precisions: [U256; 2],
        price_scale: U256,
        d: U256,
        ann: U256,
        gamma: U256,
        mid_fee: U256,
        out_fee: U256,
        fee_gamma: U256,
    },

    /// TwoCryptoNG pools using StableSwap MATH (v0.1.0).
    /// CryptoSwap interface but StableSwap invariant (gamma ignored).
    /// Detect via: pool.MATH().version() == "v0.1.0"
    TwoCryptoStable {
        balances: [U256; 2],
        precisions: [U256; 2],
        price_scale: U256,
        d: U256,
        ann: U256,
        mid_fee: U256,
        out_fee: U256,
        fee_gamma: U256,
    },

    /// Legacy 3-coin CryptoSwap (tricrypto2).
    /// Newton iteration for y.
    TriCryptoV1 {
        balances: [U256; 3],
        precisions: [U256; 3],
        price_scale: [U256; 2],
        d: U256,
        ann: U256,
        gamma: U256,
        mid_fee: U256,
        out_fee: U256,
        fee_gamma: U256,
    },

    /// Next-gen 3-coin CryptoSwap (tricrypto-ng).
    /// Hybrid cubic + Newton for y.
    TriCryptoNG {
        balances: [U256; 3],
        precisions: [U256; 3],
        price_scale: [U256; 2],
        d: U256,
        ann: U256,
        gamma: U256,
        mid_fee: U256,
        out_fee: U256,
        fee_gamma: U256,
    },
}

impl Pool {
    /// Compute output amount for swapping `dx` of coin `i` into coin `j`.
    ///
    /// Returns `None` if the swap would fail (zero input, out of range, etc.).
    /// Matches Curve's on-chain `get_dy` at wei-level precision.
    ///
    /// # Arguments
    /// * `i` — index of input coin
    /// * `j` — index of output coin
    /// * `dx` — amount of coin `i` to swap (in coin's native decimals)
    pub fn get_amount_out(&self, i: usize, j: usize, dx: U256) -> Option<U256> {
        match self {
            Pool::StableSwapV0 {
                balances,
                rates,
                amp,
                fee,
            } => swap::stableswap_v0::get_amount_out(balances, rates, *amp, *fee, i, j, dx),
            Pool::StableSwapV1 {
                balances,
                rates,
                amp,
                fee,
            } => swap::stableswap_v1::get_amount_out(balances, rates, *amp, *fee, i, j, dx),
            Pool::StableSwapV2 {
                balances,
                rates,
                amp,
                fee,
            } => swap::stableswap_v2::get_amount_out(balances, rates, *amp, *fee, i, j, dx),
            Pool::StableSwapALend {
                balances,
                precision_mul,
                amp,
                fee,
                offpeg_fee_multiplier,
            } => swap::stableswap_alend::get_amount_out(
                balances,
                precision_mul,
                *amp,
                *fee,
                *offpeg_fee_multiplier,
                i,
                j,
                dx,
            ),
            Pool::StableSwapNG {
                balances,
                rates,
                amp,
                fee,
                offpeg_fee_multiplier,
            } => swap::stableswap_ng::get_amount_out(
                balances,
                rates,
                *amp,
                *fee,
                *offpeg_fee_multiplier,
                i,
                j,
                dx,
            ),
            Pool::StableSwapMeta {
                balances,
                rates,
                amp,
                fee,
            } => swap::stableswap_meta::get_amount_out(balances, rates, *amp, *fee, i, j, dx),
            Pool::TwoCryptoV1 {
                balances,
                precisions,
                price_scale,
                d,
                ann,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
            } => swap::twocrypto_v1::get_amount_out(
                balances,
                precisions,
                *price_scale,
                *d,
                *ann,
                *gamma,
                *mid_fee,
                *out_fee,
                *fee_gamma,
                i,
                j,
                dx,
            ),
            Pool::TwoCryptoNG {
                balances,
                precisions,
                price_scale,
                d,
                ann,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
            } => swap::twocrypto_ng::get_amount_out(
                balances,
                precisions,
                *price_scale,
                *d,
                *ann,
                *gamma,
                *mid_fee,
                *out_fee,
                *fee_gamma,
                i,
                j,
                dx,
            ),
            Pool::TwoCryptoStable {
                balances,
                precisions,
                price_scale,
                d,
                ann,
                mid_fee,
                out_fee,
                fee_gamma,
            } => swap::twocrypto_stable::get_amount_out(
                balances,
                precisions,
                *price_scale,
                *d,
                *ann,
                *mid_fee,
                *out_fee,
                *fee_gamma,
                i,
                j,
                dx,
            ),
            Pool::TriCryptoV1 {
                balances,
                precisions,
                price_scale,
                d,
                ann,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
            } => swap::tricrypto_v1::get_amount_out(
                balances,
                precisions,
                price_scale,
                *d,
                *ann,
                *gamma,
                *mid_fee,
                *out_fee,
                *fee_gamma,
                i,
                j,
                dx,
            ),
            Pool::TriCryptoNG {
                balances,
                precisions,
                price_scale,
                d,
                ann,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
            } => swap::tricrypto_ng::get_amount_out(
                balances,
                precisions,
                price_scale,
                *d,
                *ann,
                *gamma,
                *mid_fee,
                *out_fee,
                *fee_gamma,
                i,
                j,
                dx,
            ),
        }
    }

    /// Compute minimum input of coin `i` to receive at least `desired_output` of coin `j`.
    ///
    /// Returns `None` if the swap is not feasible.
    /// Guaranteed: `get_amount_out(i, j, get_amount_in(i, j, dy)) >= dy`.
    pub fn get_amount_in(&self, i: usize, j: usize, desired_output: U256) -> Option<U256> {
        match self {
            Pool::StableSwapV0 {
                balances,
                rates,
                amp,
                fee,
            } => swap::stableswap_v0::get_amount_in(
                balances,
                rates,
                *amp,
                *fee,
                i,
                j,
                desired_output,
            ),
            Pool::StableSwapV1 {
                balances,
                rates,
                amp,
                fee,
            } => swap::stableswap_v1::get_amount_in(
                balances,
                rates,
                *amp,
                *fee,
                i,
                j,
                desired_output,
            ),
            Pool::StableSwapV2 {
                balances,
                rates,
                amp,
                fee,
            } => swap::stableswap_v2::get_amount_in(
                balances,
                rates,
                *amp,
                *fee,
                i,
                j,
                desired_output,
            ),
            Pool::StableSwapALend {
                balances,
                precision_mul,
                amp,
                fee,
                offpeg_fee_multiplier,
            } => swap::stableswap_alend::get_amount_in(
                balances,
                precision_mul,
                *amp,
                *fee,
                *offpeg_fee_multiplier,
                i,
                j,
                desired_output,
            ),
            Pool::StableSwapNG {
                balances,
                rates,
                amp,
                fee,
                offpeg_fee_multiplier,
            } => swap::stableswap_ng::get_amount_in(
                balances,
                rates,
                *amp,
                *fee,
                *offpeg_fee_multiplier,
                i,
                j,
                desired_output,
            ),
            Pool::StableSwapMeta {
                balances,
                rates,
                amp,
                fee,
            } => swap::stableswap_meta::get_amount_in(
                balances,
                rates,
                *amp,
                *fee,
                i,
                j,
                desired_output,
            ),
            Pool::TwoCryptoV1 {
                balances,
                precisions,
                price_scale,
                d,
                ann,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
            } => swap::twocrypto_v1::get_amount_in(
                balances,
                precisions,
                *price_scale,
                *d,
                *ann,
                *gamma,
                *mid_fee,
                *out_fee,
                *fee_gamma,
                i,
                j,
                desired_output,
            ),
            Pool::TwoCryptoNG {
                balances,
                precisions,
                price_scale,
                d,
                ann,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
            } => swap::twocrypto_ng::get_amount_in(
                balances,
                precisions,
                *price_scale,
                *d,
                *ann,
                *gamma,
                *mid_fee,
                *out_fee,
                *fee_gamma,
                i,
                j,
                desired_output,
            ),
            Pool::TwoCryptoStable {
                balances,
                precisions,
                price_scale,
                d,
                ann,
                mid_fee,
                out_fee,
                fee_gamma,
            } => swap::twocrypto_stable::get_amount_in(
                balances,
                precisions,
                *price_scale,
                *d,
                *ann,
                *mid_fee,
                *out_fee,
                *fee_gamma,
                i,
                j,
                desired_output,
            ),
            Pool::TriCryptoV1 {
                balances,
                precisions,
                price_scale,
                d,
                ann,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
            } => swap::tricrypto_v1::get_amount_in(
                balances,
                precisions,
                price_scale,
                *d,
                *ann,
                *gamma,
                *mid_fee,
                *out_fee,
                *fee_gamma,
                i,
                j,
                desired_output,
            ),
            Pool::TriCryptoNG {
                balances,
                precisions,
                price_scale,
                d,
                ann,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
            } => swap::tricrypto_ng::get_amount_in(
                balances,
                precisions,
                price_scale,
                *d,
                *ann,
                *gamma,
                *mid_fee,
                *out_fee,
                *fee_gamma,
                i,
                j,
                desired_output,
            ),
        }
    }

    /// Marginal spot price dy/dx as `(numerator, denominator)`, fee-inclusive.
    ///
    /// For a small swap of coin `i` → coin `j`, the price is approximately
    /// `numerator / denominator` in units of coin `j` per coin `i`.
    pub fn spot_price(&self, i: usize, j: usize) -> Option<(U256, U256)> {
        match self {
            Pool::StableSwapV0 {
                balances,
                rates,
                amp,
                fee,
            } => swap::stableswap_v0::spot_price(balances, rates, *amp, *fee, i, j),
            Pool::StableSwapV1 {
                balances,
                rates,
                amp,
                fee,
            } => swap::stableswap_v1::spot_price(balances, rates, *amp, *fee, i, j),
            Pool::StableSwapV2 {
                balances,
                rates,
                amp,
                fee,
            } => swap::stableswap_v2::spot_price(balances, rates, *amp, *fee, i, j),
            Pool::StableSwapALend {
                balances,
                precision_mul,
                amp,
                fee,
                offpeg_fee_multiplier,
            } => swap::stableswap_alend::spot_price(
                balances,
                precision_mul,
                *amp,
                *fee,
                *offpeg_fee_multiplier,
                i,
                j,
            ),
            Pool::StableSwapNG {
                balances,
                rates,
                amp,
                fee,
                offpeg_fee_multiplier,
            } => swap::stableswap_ng::spot_price(
                balances,
                rates,
                *amp,
                *fee,
                *offpeg_fee_multiplier,
                i,
                j,
            ),
            Pool::StableSwapMeta {
                balances,
                rates,
                amp,
                fee,
            } => swap::stableswap_meta::spot_price(balances, rates, *amp, *fee, i, j),
            Pool::TwoCryptoV1 {
                balances,
                precisions,
                price_scale,
                d,
                ann,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
            } => swap::twocrypto_v1::spot_price(
                balances,
                precisions,
                *price_scale,
                *d,
                *ann,
                *gamma,
                *mid_fee,
                *out_fee,
                *fee_gamma,
                i,
                j,
            ),
            Pool::TwoCryptoNG {
                balances,
                precisions,
                price_scale,
                d,
                ann,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
            } => swap::twocrypto_ng::spot_price(
                balances,
                precisions,
                *price_scale,
                *d,
                *ann,
                *gamma,
                *mid_fee,
                *out_fee,
                *fee_gamma,
                i,
                j,
            ),
            Pool::TwoCryptoStable {
                balances,
                precisions,
                price_scale,
                d,
                ann,
                mid_fee,
                out_fee,
                fee_gamma,
            } => swap::twocrypto_stable::spot_price(
                balances,
                precisions,
                *price_scale,
                *d,
                *ann,
                *mid_fee,
                *out_fee,
                *fee_gamma,
                i,
                j,
            ),
            Pool::TriCryptoV1 {
                balances,
                precisions,
                price_scale,
                d,
                ann,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
            } => swap::tricrypto_v1::spot_price(
                balances,
                precisions,
                price_scale,
                *d,
                *ann,
                *gamma,
                *mid_fee,
                *out_fee,
                *fee_gamma,
                i,
                j,
            ),
            Pool::TriCryptoNG {
                balances,
                precisions,
                price_scale,
                d,
                ann,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
            } => swap::tricrypto_ng::spot_price(
                balances,
                precisions,
                price_scale,
                *d,
                *ann,
                *gamma,
                *mid_fee,
                *out_fee,
                *fee_gamma,
                i,
                j,
            ),
        }
    }
}
