//! Property-based fuzz tests for get_amount_in and spot_price.
//! No RPC required — pure math tests using realistic pool parameters.
//!
//! Run all:
//!   cargo test --features swap --test fuzz_properties -- --nocapture
//!
//! Run specific:
//!   cargo test --features swap --test fuzz_properties -- roundtrip_stableswap_v2 --nocapture

#![cfg(feature = "swap")]

use alloy_primitives::U256;

// ── Deterministic PRNG ──────────────────────────────────────────────────────

fn splitmix64(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = *seed;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

fn generate_amounts(n: usize, max: U256, seed: u64) -> Vec<U256> {
    if max.is_zero() || n == 0 {
        return vec![];
    }
    let mut s = seed;
    let mut out = Vec::with_capacity(n);
    let max_f64 = max.to_string().parse::<f64>().unwrap_or(1e30);
    let ln_max = max_f64.ln();
    // Edge cases first
    out.push(U256::from(1u64));
    out.push(max / U256::from(1000u64));
    out.push(max / U256::from(10u64));
    out.push(max / U256::from(2u64));
    let remaining = n.saturating_sub(out.len());
    for _ in 0..remaining {
        let r = splitmix64(&mut s);
        let t = (r as f64) / (u64::MAX as f64);
        let val = (t * ln_max).exp();
        let v = U256::from(val.min(1e38) as u128)
            .max(U256::from(1u64))
            .min(max);
        out.push(v);
    }
    out
}

fn prop_iterations() -> usize {
    std::env::var("PROP_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200)
}

// ═══════════════════════════════════════════════════════════════════════════
// ROUNDTRIP TESTS: get_amount_out(get_amount_in(desired_dy)) >= desired_dy
// ═══════════════════════════════════════════════════════════════════════════

// ── StableSwap V0 ───────────────────────────────────────────────────────────

#[test]
fn roundtrip_stableswap_v0() {
    use curve_math::swap::stableswap_v0::{get_amount_in, get_amount_out};
    let wad = U256::from(1_000_000_000_000_000_000u128);
    let balances = [
        U256::from(5_000_000u64) * wad,
        U256::from(5_000_000_000_000u64), // 6-dec
        U256::from(5_000_000_000_000u64), // 6-dec
        U256::from(5_000_000u64) * wad,
    ];
    let rate18 = wad;
    let rate6 = U256::from(1_000_000_000_000_000_000_000_000_000_000u128);
    let rates = [rate18, rate6, rate6, rate18];
    let amp = U256::from(100u64);
    let fee = U256::from(4_000_000u64);
    let n = prop_iterations();
    let mut passed = 0u64;
    let mut skipped = 0u64;
    for desired_dy in generate_amounts(n, balances[1] / U256::from(2u64), 42) {
        let dx = match get_amount_in(&balances, &rates, amp, fee, 0, 1, desired_dy) {
            Some(v) => v,
            None => {
                skipped += 1;
                continue;
            }
        };
        let actual_dy = match get_amount_out(&balances, &rates, amp, fee, 0, 1, dx) {
            Some(v) => v,
            None => {
                skipped += 1;
                continue;
            }
        };
        assert!(
            actual_dy >= desired_dy,
            "V0 roundtrip: got {actual_dy} < desired {desired_dy}"
        );
        passed += 1;
    }
    println!("roundtrip_stableswap_v0: {passed} passed, {skipped} skipped");
    assert!(passed > 0);
}

// ── StableSwap V1 ───────────────────────────────────────────────────────────

#[test]
fn roundtrip_stableswap_v1() {
    use curve_math::swap::stableswap_v1::{get_amount_in, get_amount_out};
    let wad = U256::from(1_000_000_000_000_000_000u128);
    let balances = [
        U256::from(50_000_000u64) * wad,
        U256::from(50_000_000_000_000u64),
        U256::from(50_000_000_000_000u64),
    ];
    let rate18 = wad;
    let rate6 = U256::from(1_000_000_000_000_000_000_000_000_000_000u128);
    let rates = [rate18, rate6, rate6];
    let amp = U256::from(2000u64);
    let fee = U256::from(1_000_000u64);
    let n = prop_iterations();
    let mut passed = 0u64;
    let mut skipped = 0u64;
    for desired_dy in generate_amounts(n, balances[1] / U256::from(2u64), 43) {
        let dx = match get_amount_in(&balances, &rates, amp, fee, 0, 1, desired_dy) {
            Some(v) => v,
            None => {
                skipped += 1;
                continue;
            }
        };
        let actual_dy = match get_amount_out(&balances, &rates, amp, fee, 0, 1, dx) {
            Some(v) => v,
            None => {
                skipped += 1;
                continue;
            }
        };
        assert!(
            actual_dy >= desired_dy,
            "V1 roundtrip: got {actual_dy} < desired {desired_dy}"
        );
        passed += 1;
    }
    println!("roundtrip_stableswap_v1: {passed} passed, {skipped} skipped");
    assert!(passed > 0);
}

// ── StableSwap V2 ───────────────────────────────────────────────────────────

#[test]
fn roundtrip_stableswap_v2() {
    use curve_math::core::stableswap_v2::A_PRECISION;
    use curve_math::swap::stableswap_v2::{get_amount_in, get_amount_out};
    let wad = U256::from(1_000_000_000_000_000_000u128);
    let balances = [
        U256::from(10_000_000u64) * wad,
        U256::from(10_000_000_000_000u64),
    ];
    let rate18 = wad;
    let rate6 = U256::from(1_000_000_000_000_000_000_000_000_000_000u128);
    let rates = [rate18, rate6];
    let amp = U256::from(200u64 * A_PRECISION as u64);
    let fee = U256::from(4_000_000u64);
    let n = prop_iterations();
    let mut passed = 0u64;
    let mut skipped = 0u64;
    // Both directions
    for desired_dy in generate_amounts(n / 2, balances[1] / U256::from(2u64), 44) {
        let dx = match get_amount_in(&balances, &rates, amp, fee, 0, 1, desired_dy) {
            Some(v) => v,
            None => {
                skipped += 1;
                continue;
            }
        };
        let actual_dy = match get_amount_out(&balances, &rates, amp, fee, 0, 1, dx) {
            Some(v) => v,
            None => {
                skipped += 1;
                continue;
            }
        };
        assert!(
            actual_dy >= desired_dy,
            "V2 0→1 roundtrip: got {actual_dy} < desired {desired_dy}"
        );
        passed += 1;
    }
    for desired_dy in generate_amounts(n / 2, balances[0] / U256::from(2u64), 45) {
        let dx = match get_amount_in(&balances, &rates, amp, fee, 1, 0, desired_dy) {
            Some(v) => v,
            None => {
                skipped += 1;
                continue;
            }
        };
        let actual_dy = match get_amount_out(&balances, &rates, amp, fee, 1, 0, dx) {
            Some(v) => v,
            None => {
                skipped += 1;
                continue;
            }
        };
        assert!(
            actual_dy >= desired_dy,
            "V2 1→0 roundtrip: got {actual_dy} < desired {desired_dy}"
        );
        passed += 1;
    }
    println!("roundtrip_stableswap_v2: {passed} passed, {skipped} skipped");
    assert!(passed > 0);
}

// ── TwoCrypto V1 ────────────────────────────────────────────────────────────

#[test]
fn roundtrip_twocrypto_v1() {
    use curve_math::core::twocrypto_v1::{A_MULTIPLIER, WAD};
    use curve_math::swap::twocrypto_v1::{get_amount_in, get_amount_out};
    let wad = U256::from(WAD);
    let balances = [U256::from(5000u64) * wad, U256::from(5000u64) * wad];
    let precisions = [U256::from(1u64), U256::from(1u64)];
    let price_scale = wad;
    let d = U256::from(10000u64) * wad;
    let ann = U256::from(540_000u64) * U256::from(A_MULTIPLIER as u64);
    let gamma = U256::from(28_000_000_000_000u64);
    let mid_fee = U256::from(3_000_000u64);
    let out_fee = U256::from(30_000_000u64);
    let fee_gamma = U256::from(230_000_000_000_000u64);
    let n = prop_iterations();
    let mut passed = 0u64;
    let mut skipped = 0u64;
    // CryptoSwap get_amount_in has a linear search loop, so use fewer iterations
    for desired_dy in generate_amounts(n / 4, balances[1] / U256::from(4u64), 50) {
        let dx = match get_amount_in(
            &balances,
            &precisions,
            price_scale,
            d,
            ann,
            gamma,
            mid_fee,
            out_fee,
            fee_gamma,
            0,
            1,
            desired_dy,
        ) {
            Some(v) => v,
            None => {
                skipped += 1;
                continue;
            }
        };
        let actual_dy = match get_amount_out(
            &balances,
            &precisions,
            price_scale,
            d,
            ann,
            gamma,
            mid_fee,
            out_fee,
            fee_gamma,
            0,
            1,
            dx,
        ) {
            Some(v) => v,
            None => {
                skipped += 1;
                continue;
            }
        };
        assert!(
            actual_dy >= desired_dy,
            "TwoCryptoV1 roundtrip: got {actual_dy} < desired {desired_dy}"
        );
        passed += 1;
    }
    println!("roundtrip_twocrypto_v1: {passed} passed, {skipped} skipped");
    assert!(passed > 0);
}

// ═══════════════════════════════════════════════════════════════════════════
// SPOT PRICE CONSISTENCY: spot_price ≈ get_amount_out(tiny_dx) / tiny_dx
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn spot_price_stableswap_v2() {
    use curve_math::core::stableswap_v2::A_PRECISION;
    use curve_math::swap::stableswap_v2::{get_amount_out, spot_price};
    let wad = U256::from(1_000_000_000_000_000_000u128);

    // Test with IMBALANCED pool to catch the formula bug
    let balances = [
        U256::from(20_000_000u64) * wad,
        U256::from(5_000_000_000_000u64),
    ];
    let rate18 = wad;
    let rate6 = U256::from(1_000_000_000_000_000_000_000_000_000_000u128);
    let rates = [rate18, rate6];
    let amp = U256::from(200u64 * A_PRECISION as u64);
    let fee = U256::from(4_000_000u64);

    let (num, den) = spot_price(&balances, &rates, amp, fee, 0, 1).expect("price");
    // Numerical price: get_amount_out with small dx
    let dx = U256::from(1_000_000_000_000_000u128); // 0.001 token
    let dy = get_amount_out(&balances, &rates, amp, fee, 0, 1, dx).expect("dy");
    // spot_price = num/den, numerical = dy/dx
    // Cross-multiply: num * dx ≈ dy * den (within 1%)
    let lhs = num * dx;
    let rhs = dy * den;
    let diff = if lhs > rhs { lhs - rhs } else { rhs - lhs };
    assert!(
        diff * U256::from(100u64) < rhs,
        "V2 spot_price inconsistent: num={num}, den={den}, dx={dx}, dy={dy}, diff%={:.4}",
        diff.to_string().parse::<f64>().unwrap_or(0.0)
            / rhs.to_string().parse::<f64>().unwrap_or(1.0)
            * 100.0
    );
    println!("spot_price_stableswap_v2: OK (imbalanced pool, <1% error)");
}

#[test]
fn spot_price_stableswap_v1_imbalanced() {
    use curve_math::swap::stableswap_v1::{get_amount_out, spot_price};
    let wad = U256::from(1_000_000_000_000_000_000u128);
    // 3pool imbalanced: lots of DAI, less USDC/USDT
    let balances = [
        U256::from(100_000_000u64) * wad,
        U256::from(20_000_000_000_000u64),
        U256::from(30_000_000_000_000u64),
    ];
    let rate18 = wad;
    let rate6 = U256::from(1_000_000_000_000_000_000_000_000_000_000u128);
    let rates = [rate18, rate6, rate6];
    let amp = U256::from(2000u64);
    let fee = U256::from(1_000_000u64);

    // Test all 6 pairs
    let pairs = [(0, 1), (0, 2), (1, 0), (1, 2), (2, 0), (2, 1)];
    for (i, j) in pairs {
        let (num, den) = spot_price(&balances, &rates, amp, fee, i, j).expect("price");
        // Small dx relative to balance
        let dx = balances[i] / U256::from(1_000_000u64);
        if dx.is_zero() {
            continue;
        }
        let dy = match get_amount_out(&balances, &rates, amp, fee, i, j, dx) {
            Some(v) => v,
            None => continue,
        };
        if dy.is_zero() {
            continue;
        }
        let lhs = num * dx;
        let rhs = dy * den;
        let diff = if lhs > rhs { lhs - rhs } else { rhs - lhs };
        assert!(
            diff * U256::from(100u64) < rhs,
            "V1 spot_price {i}→{j} inconsistent: >1% error"
        );
    }
    println!("spot_price_stableswap_v1_imbalanced: OK (all 6 pairs, <1% error)");
}

#[test]
fn spot_price_stableswap_ng_imbalanced() {
    use curve_math::core::stableswap_ng::A_PRECISION;
    use curve_math::swap::stableswap_ng::{get_amount_out, spot_price};
    let wad = U256::from(1_000_000_000_000_000_000u128);
    // Moderately imbalanced 2-coin NG pool (dynamic fee shifts with trade size)
    let balances = [
        U256::from(12_000_000u64) * wad,
        U256::from(10_000_000u64) * wad,
    ];
    let rates = [wad, wad];
    let amp = U256::from(400u64 * A_PRECISION as u64);
    let fee = U256::from(4_000_000u64);
    let offpeg = U256::from(20_000_000_000u64);

    for (i, j) in [(0, 1), (1, 0)] {
        let (num, den) = spot_price(&balances, &rates, amp, fee, offpeg, i, j).expect("price");
        let dx = balances[i] / U256::from(1_000_000u64);
        let dy = get_amount_out(&balances, &rates, amp, fee, offpeg, i, j, dx).expect("dy");
        let lhs = num * dx;
        let rhs = dy * den;
        let diff = if lhs > rhs { lhs - rhs } else { rhs - lhs };
        assert!(
            diff * U256::from(100u64) < rhs,
            "NG spot_price {i}→{j} inconsistent: >1% error"
        );
    }
    println!("spot_price_stableswap_ng_imbalanced: OK (both directions, <1% error)");
}
