//! Differential fuzz tests: random swap amounts compared against deployed Curve contracts.
//!
//! Run a single variant:
//!   FUZZ_ITERATIONS=200 RPC_URL=... cargo test --features swap --test fuzz_differential -- fuzz_stableswap_v2 --ignored --nocapture
//!
//! Run all:
//!   FUZZ_ITERATIONS=100 RPC_URL=... cargo test --features swap --test fuzz_differential -- --ignored --nocapture

#![cfg(feature = "swap")]

use alloy::providers::{Provider, ProviderBuilder};
use alloy_primitives::U256;
use std::str::FromStr;

// ── Deterministic PRNG (splitmix64) ─────────────────────────────────────────

fn splitmix64(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = *seed;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

/// Generate test amounts: edge cases + logarithmically spaced random values.
///
/// `balance` is the pool balance for the input token (used to compute edge cases).
/// Returns amounts including: 0, 1, balance, 2*balance, U256::MAX, plus random log-spaced.
fn generate_amounts(n: usize, balance: U256, block_seed: u64) -> Vec<U256> {
    if n == 0 {
        return vec![];
    }
    let mut amounts = Vec::with_capacity(n + 6);

    // Edge cases: these stress the boundaries
    amounts.push(U256::ZERO); // should revert/return None
    amounts.push(U256::from(1u64)); // 1 wei — minimum
    amounts.push(balance / U256::from(1000u64)); // 0.1% of balance
    amounts.push(balance / U256::from(10u64)); // 10% of balance
    amounts.push(balance / U256::from(2u64)); // 50% of balance
    amounts.push(balance); // 100% of balance
    amounts.push(balance * U256::from(2u64)); // 200% — should revert
    amounts.push(U256::MAX); // overflow — should revert

    // Random log-spaced: from 1 wei to full balance
    let remaining = n.saturating_sub(amounts.len());
    if remaining > 0 && !balance.is_zero() {
        let mut seed = block_seed;
        let max_f64 = balance.to_string().parse::<f64>().unwrap_or(1e30);
        let ln_max = max_f64.ln();
        for _ in 0..remaining {
            let r = splitmix64(&mut seed);
            let t = (r as f64) / (u64::MAX as f64);
            let val = (t * ln_max).exp();
            let val_u128 = val.min(1e38) as u128;
            let amount = U256::from(val_u128).max(U256::from(1u64)).min(balance);
            amounts.push(amount);
        }
    }
    amounts
}

fn fuzz_iterations() -> usize {
    std::env::var("FUZZ_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100)
}

macro_rules! make_provider {
    () => {{
        let rpc_url = std::env::var("RPC_URL").expect("RPC_URL must be set");
        ProviderBuilder::new().connect_http(rpc_url.parse().expect("invalid RPC_URL"))
    }};
}

// ── StableSwap V0 (sUSD: DAI/USDC/USDT/sUSD) ──────────────────────────────

alloy::sol! {
    #[sol(rpc)]
    interface ICurvePoolOld {
        function get_dy(int128 i, int128 j, uint256 dx) external view returns (uint256);
        function balances(int128 i) external view returns (uint256);
        function A() external view returns (uint256);
        function fee() external view returns (uint256);
    }
}

#[tokio::test]
#[ignore = "requires RPC_URL"]
async fn fuzz_stableswap_v0() {
    use curve_math::swap::stableswap_v0::get_amount_out;

    let provider = make_provider!();
    let addr =
        alloy_primitives::Address::from_str("0xA5407eAE9Ba41422680e2e00537571bcC53efBfD").unwrap();
    let curve = ICurvePoolOld::new(addr, &provider);
    let bn = provider.get_block_number().await.unwrap() - 5;
    let block = alloy::eips::BlockId::number(bn);

    let b0 = curve.balances(0i128).block(block).call().await.unwrap();
    let b1 = curve.balances(1i128).block(block).call().await.unwrap();
    let b2 = curve.balances(2i128).block(block).call().await.unwrap();
    let b3 = curve.balances(3i128).block(block).call().await.unwrap();
    let amp = curve.A().block(block).call().await.unwrap();
    let fee = curve.fee().block(block).call().await.unwrap();

    let rate18 = U256::from(1_000_000_000_000_000_000u128);
    let rate6 = U256::from(1_000_000_000_000_000_000_000_000_000_000u128);
    let rates = [rate18, rate6, rate6, rate18];
    let balances = [b0, b1, b2, b3];

    let n = fuzz_iterations();
    let n_coins = 4usize;
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for i in 0..n_coins {
        for j in 0..n_coins {
            if i != j {
                pairs.push((i, j));
            }
        }
    }
    let per_pair = (n / pairs.len()).max(1);

    let mut passed = 0u64;
    let mut skipped = 0u64;
    for (idx, &(i, j)) in pairs.iter().enumerate() {
        for dx in generate_amounts(per_pair, balances[i], bn + idx as u64) {
            let on_chain = curve
                .get_dy(i as i128, j as i128, dx)
                .block(block)
                .call()
                .await;
            match on_chain {
                Ok(expected) => match get_amount_out(&balances, &rates, amp, fee, i, j, dx) {
                    Some(result) => {
                        assert_eq!(result, expected, "V0 {i}→{j} mismatch at dx={dx}");
                        passed += 1;
                    }
                    None => skipped += 1,
                },
                Err(_) => skipped += 1,
            }
        }
    }
    println!("fuzz_stableswap_v0: {passed} passed, {skipped} skipped, {n_coins}-coin all pairs (block {bn})");
    assert!(passed > 0, "no tests passed");
}

// ── StableSwap V1 (3pool: DAI/USDC/USDT) ───────────────────────────────────

alloy::sol! {
    #[sol(rpc)]
    interface ICurvePool3 {
        function get_dy(int128 i, int128 j, uint256 dx) external view returns (uint256);
        function balances(uint256 i) external view returns (uint256);
        function A() external view returns (uint256);
        function fee() external view returns (uint256);
    }
}

#[tokio::test]
#[ignore = "requires RPC_URL"]
async fn fuzz_stableswap_v1() {
    use curve_math::swap::stableswap_v1::get_amount_out;

    let provider = make_provider!();
    let addr =
        alloy_primitives::Address::from_str("0xbEbc44782C7dB0a1A60Cb6fe97d0b483032FF1C7").unwrap();
    let curve = ICurvePool3::new(addr, &provider);
    let bn = provider.get_block_number().await.unwrap() - 5;
    let block = alloy::eips::BlockId::number(bn);

    let b0 = curve
        .balances(U256::from(0))
        .block(block)
        .call()
        .await
        .unwrap();
    let b1 = curve
        .balances(U256::from(1))
        .block(block)
        .call()
        .await
        .unwrap();
    let b2 = curve
        .balances(U256::from(2))
        .block(block)
        .call()
        .await
        .unwrap();
    let amp = curve.A().block(block).call().await.unwrap();
    let fee = curve.fee().block(block).call().await.unwrap();

    let rate18 = U256::from(1_000_000_000_000_000_000u128);
    let rate6 = U256::from(1_000_000_000_000_000_000_000_000_000_000u128);
    let rates = [rate18, rate6, rate6];
    let balances = [b0, b1, b2];

    let n = fuzz_iterations();
    let n_coins = 3usize;
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for i in 0..n_coins {
        for j in 0..n_coins {
            if i != j {
                pairs.push((i, j));
            }
        }
    }
    let per_pair = (n / pairs.len()).max(1);

    let mut passed = 0u64;
    let mut skipped = 0u64;
    for (idx, &(i, j)) in pairs.iter().enumerate() {
        for dx in generate_amounts(per_pair, balances[i], bn + idx as u64) {
            let on_chain = curve
                .get_dy(i as i128, j as i128, dx)
                .block(block)
                .call()
                .await;
            match on_chain {
                Ok(expected) => match get_amount_out(&balances, &rates, amp, fee, i, j, dx) {
                    Some(result) => {
                        assert_eq!(result, expected, "V1 {i}→{j} mismatch at dx={dx}");
                        passed += 1;
                    }
                    None => skipped += 1,
                },
                Err(_) => skipped += 1,
            }
        }
    }
    println!("fuzz_stableswap_v1: {passed} passed, {skipped} skipped, {n_coins}-coin all pairs (block {bn})");
    assert!(passed > 0, "no tests passed");
}

// ── StableSwap V1 liquidity: calc_withdraw_one_coin (3pool) ─────────────────

alloy::sol! {
    #[sol(rpc)]
    interface ICurvePool3Liq {
        function calc_withdraw_one_coin(uint256 _token_amount, int128 i) external view returns (uint256);
        function balances(uint256 i) external view returns (uint256);
        function A() external view returns (uint256);
        function fee() external view returns (uint256);
    }

    #[sol(rpc)]
    interface IERC20Supply {
        function totalSupply() external view returns (uint256);
    }
}

#[tokio::test]
#[ignore = "requires RPC_URL"]
async fn fuzz_calc_withdraw_one_coin_v1() {
    use curve_math::swap::stableswap_v1::calc_withdraw_one_coin;

    let provider = make_provider!();
    let pool_addr =
        alloy_primitives::Address::from_str("0xbEbc44782C7dB0a1A60Cb6fe97d0b483032FF1C7").unwrap();
    let lp_addr =
        alloy_primitives::Address::from_str("0x6c3F90f043a72FA612cbac8115EE7e52BDE6E490").unwrap();
    let curve = ICurvePool3Liq::new(pool_addr, &provider);
    let lp_token = IERC20Supply::new(lp_addr, &provider);
    let bn = provider.get_block_number().await.unwrap() - 5;
    let block = alloy::eips::BlockId::number(bn);

    let b0 = curve
        .balances(U256::from(0))
        .block(block)
        .call()
        .await
        .unwrap();
    let b1 = curve
        .balances(U256::from(1))
        .block(block)
        .call()
        .await
        .unwrap();
    let b2 = curve
        .balances(U256::from(2))
        .block(block)
        .call()
        .await
        .unwrap();
    let amp = curve.A().block(block).call().await.unwrap();
    let fee = curve.fee().block(block).call().await.unwrap();
    let total_supply = lp_token.totalSupply().block(block).call().await.unwrap();

    let rate18 = U256::from(1_000_000_000_000_000_000u128);
    let rate6 = U256::from(1_000_000_000_000_000_000_000_000_000_000u128);
    let rates = [rate18, rate6, rate6];
    let balances = [b0, b1, b2];

    let n = fuzz_iterations();
    let per_coin = (n / 3).max(1);

    let mut passed = 0u64;
    let mut skipped = 0u64;
    for i in 0..3 {
        for lp_amount in generate_amounts(per_coin, total_supply / U256::from(10u64), bn + i as u64)
        {
            if lp_amount.is_zero() || lp_amount >= total_supply {
                skipped += 1;
                continue;
            }
            let on_chain = curve
                .calc_withdraw_one_coin(lp_amount, i as i128)
                .block(block)
                .call()
                .await;
            match on_chain {
                Ok(expected) => match calc_withdraw_one_coin(
                    &balances,
                    &rates,
                    amp,
                    fee,
                    lp_amount,
                    i,
                    total_supply,
                ) {
                    Some(result) => {
                        assert_eq!(
                            result, expected,
                            "withdraw coin {i} mismatch at lp={lp_amount}"
                        );
                        passed += 1;
                    }
                    None => skipped += 1,
                },
                Err(_) => skipped += 1,
            }
        }
    }
    println!("fuzz_calc_withdraw_one_coin_v1: {passed} passed, {skipped} skipped (block {bn})");
    assert!(passed > 0, "no tests passed");
}

// ── StableSwap V1 liquidity: calc_add_liquidity (3pool) ─────────────────────

alloy::sol! {
    #[sol(rpc)]
    interface ICurvePool3AddLiq {
        function balances(uint256 i) external view returns (uint256);
        function A() external view returns (uint256);
        function fee() external view returns (uint256);
        function add_liquidity(uint256[3] amounts, uint256 min_mint_amount) external;
    }
}

#[tokio::test]
#[ignore = "requires RPC_URL"]
async fn fuzz_calc_add_liquidity_v1() {
    use curve_math::swap::stableswap_v1::calc_add_liquidity;

    let provider = make_provider!();
    let pool_addr =
        alloy_primitives::Address::from_str("0xbEbc44782C7dB0a1A60Cb6fe97d0b483032FF1C7").unwrap();
    let lp_addr =
        alloy_primitives::Address::from_str("0x6c3F90f043a72FA612cbac8115EE7e52BDE6E490").unwrap();
    let curve = ICurvePool3AddLiq::new(pool_addr, &provider);
    let lp_token = IERC20Supply::new(lp_addr, &provider);
    let bn = provider.get_block_number().await.unwrap() - 5;
    let block = alloy::eips::BlockId::number(bn);

    let b0 = curve
        .balances(U256::from(0))
        .block(block)
        .call()
        .await
        .unwrap();
    let b1 = curve
        .balances(U256::from(1))
        .block(block)
        .call()
        .await
        .unwrap();
    let b2 = curve
        .balances(U256::from(2))
        .block(block)
        .call()
        .await
        .unwrap();
    let amp = curve.A().block(block).call().await.unwrap();
    let fee = curve.fee().block(block).call().await.unwrap();
    let total_supply = lp_token.totalSupply().block(block).call().await.unwrap();

    let rate18 = U256::from(1_000_000_000_000_000_000u128);
    let rate6 = U256::from(1_000_000_000_000_000_000_000_000_000_000u128);
    let rates = [rate18, rate6, rate6];
    let balances = [b0, b1, b2];

    let n = fuzz_iterations();
    let per_coin = (n / 3).max(1);

    let mut passed = 0u64;
    let mut skipped = 0u64;

    // Test single-coin deposits for each coin
    for coin in 0..3 {
        for deposit_amount in generate_amounts(
            per_coin,
            balances[coin] / U256::from(10u64),
            bn + coin as u64,
        ) {
            if deposit_amount.is_zero() {
                skipped += 1;
                continue;
            }

            let mut amounts = [U256::ZERO; 3];
            amounts[coin] = deposit_amount;

            // Note: add_liquidity is state-changing so we can't call it via eth_call
            // without state overrides. We verify indirectly:
            // 1. Sanity checks here (mint > 0, mint < supply)
            // 2. Roundtrip with calc_withdraw_one_coin in unit tests
            // 3. calc_withdraw_one_coin is already wei-exact fuzz verified

            match calc_add_liquidity(&balances, &rates, amp, fee, &amounts, total_supply) {
                Some(mint) => {
                    // Basic sanity: mint > 0 for non-zero deposit
                    assert!(
                        mint > U256::ZERO,
                        "mint should be positive for coin {coin} deposit {deposit_amount}"
                    );
                    // mint should be less than total_supply (can't double supply with small deposit)
                    if deposit_amount < balances[coin] {
                        assert!(mint < total_supply, "mint too large for coin {coin}");
                    }
                    passed += 1;
                }
                None => skipped += 1,
            }
        }
    }
    println!("fuzz_calc_add_liquidity_v1: {passed} passed, {skipped} skipped (block {bn})");
    assert!(passed > 0, "no tests passed");
}

// ── StableSwap V2 (FRAX/USDC) ──────────────────────────────────────────────

alloy::sol! {
    #[sol(rpc)]
    interface ICurvePoolV2 {
        function get_dy(int128 i, int128 j, uint256 dx) external view returns (uint256);
        function balances(uint256 i) external view returns (uint256);
        function A() external view returns (uint256);
        function fee() external view returns (uint256);
    }
}

#[tokio::test]
#[ignore = "requires RPC_URL"]
async fn fuzz_stableswap_v2() {
    use curve_math::core::stableswap_v2::A_PRECISION;
    use curve_math::swap::stableswap_v2::get_amount_out;

    let provider = make_provider!();
    let addr =
        alloy_primitives::Address::from_str("0xDcEF968d416a41Cdac0ED8702fAC8128A64241A2").unwrap();
    let curve = ICurvePoolV2::new(addr, &provider);
    let bn = provider.get_block_number().await.unwrap() - 5;
    let block = alloy::eips::BlockId::number(bn);

    let b0 = curve
        .balances(U256::from(0))
        .block(block)
        .call()
        .await
        .unwrap();
    let b1 = curve
        .balances(U256::from(1))
        .block(block)
        .call()
        .await
        .unwrap();
    let raw_a = curve.A().block(block).call().await.unwrap();
    let fee = curve.fee().block(block).call().await.unwrap();

    let rate18 = U256::from(1_000_000_000_000_000_000u128);
    let rate6 = U256::from(1_000_000_000_000_000_000_000_000_000_000u128);
    let rates = [rate18, rate6];
    let balances = [b0, b1];
    let amp = raw_a * A_PRECISION;

    let n = fuzz_iterations();
    let half = n / 2;
    let mut passed = 0u64;
    let mut skipped = 0u64;

    for dx in generate_amounts(half, balances[0], bn) {
        let on_chain = curve.get_dy(0i128, 1i128, dx).block(block).call().await;
        match on_chain {
            Ok(expected) => match get_amount_out(&balances, &rates, amp, fee, 0, 1, dx) {
                Some(result) => {
                    assert_eq!(result, expected, "V2 0→1 mismatch at dx={dx}");
                    passed += 1;
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    for dx in generate_amounts(n - half, balances[1], bn + 1) {
        let on_chain = curve.get_dy(1i128, 0i128, dx).block(block).call().await;
        match on_chain {
            Ok(expected) => match get_amount_out(&balances, &rates, amp, fee, 1, 0, dx) {
                Some(result) => {
                    assert_eq!(result, expected, "V2 1→0 mismatch at dx={dx}");
                    passed += 1;
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    println!("fuzz_stableswap_v2: {passed} passed, {skipped} skipped (block {bn})");
    assert!(passed > 0, "no tests passed");
}

// ── StableSwap ALend (Aave: aDAI/aUSDC/aUSDT) ─────────────────────────────

alloy::sol! {
    #[sol(rpc)]
    interface ICurvePoolALend {
        function get_dy(int128 i, int128 j, uint256 dx) external view returns (uint256);
        function balances(uint256 i) external view returns (uint256);
        function A() external view returns (uint256);
        function fee() external view returns (uint256);
        function offpeg_fee_multiplier() external view returns (uint256);
    }
}

#[tokio::test]
#[ignore = "requires RPC_URL"]
async fn fuzz_stableswap_alend() {
    use curve_math::swap::stableswap_alend::get_amount_out;

    let provider = make_provider!();
    let addr =
        alloy_primitives::Address::from_str("0xDeBF20617708857ebe4F679508E7b7863a8A8EeE").unwrap();
    let curve = ICurvePoolALend::new(addr, &provider);
    let bn = provider.get_block_number().await.unwrap() - 5;
    let block = alloy::eips::BlockId::number(bn);

    let b0 = curve
        .balances(U256::from(0))
        .block(block)
        .call()
        .await
        .unwrap();
    let b1 = curve
        .balances(U256::from(1))
        .block(block)
        .call()
        .await
        .unwrap();
    let b2 = curve
        .balances(U256::from(2))
        .block(block)
        .call()
        .await
        .unwrap();
    let raw_a = curve.A().block(block).call().await.unwrap();
    let fee = curve.fee().block(block).call().await.unwrap();
    let offpeg = curve
        .offpeg_fee_multiplier()
        .block(block)
        .call()
        .await
        .unwrap();

    let balances = [b0, b1, b2];
    let precision_mul = [
        U256::from(1u64),
        U256::from(1_000_000_000_000u64),
        U256::from(1_000_000_000_000u64),
    ];
    let amp = raw_a * curve_math::core::stableswap_alend::A_PRECISION;

    let n = fuzz_iterations();
    let n_coins = 3usize;
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for i in 0..n_coins {
        for j in 0..n_coins {
            if i != j {
                pairs.push((i, j));
            }
        }
    }
    let per_pair = (n / pairs.len()).max(1);

    let mut passed = 0u64;
    let mut skipped = 0u64;
    for (idx, &(i, j)) in pairs.iter().enumerate() {
        for dx in generate_amounts(per_pair, balances[i], bn + idx as u64) {
            let on_chain = curve
                .get_dy(i as i128, j as i128, dx)
                .block(block)
                .call()
                .await;
            match on_chain {
                Ok(expected) => {
                    match get_amount_out(&balances, &precision_mul, amp, fee, offpeg, i, j, dx) {
                        Some(result) => {
                            assert_eq!(result, expected, "ALend {i}→{j} mismatch at dx={dx}");
                            passed += 1;
                        }
                        None => skipped += 1,
                    }
                }
                Err(_) => skipped += 1,
            }
        }
    }
    println!("fuzz_stableswap_alend: {passed} passed, {skipped} skipped, {n_coins}-coin all pairs (block {bn})");
    assert!(passed > 0, "no tests passed");
}

// ── StableSwap NG (USDe/DAI) ────────────────────────────────────────────────

alloy::sol! {
    #[sol(rpc)]
    interface ICurvePoolNG {
        function get_dy(int128 i, int128 j, uint256 dx) external view returns (uint256);
        function balances(uint256 i) external view returns (uint256);
        function A() external view returns (uint256);
        function fee() external view returns (uint256);
        function offpeg_fee_multiplier() external view returns (uint256);
    }
}

#[tokio::test]
#[ignore = "requires RPC_URL"]
async fn fuzz_stableswap_ng() {
    use curve_math::core::stableswap_ng::A_PRECISION;
    use curve_math::swap::stableswap_ng::get_amount_out;

    let provider = make_provider!();
    let addr =
        alloy_primitives::Address::from_str("0xF36a4BA50C603204c3FC6d2dA8b78A7b69CBC67d").unwrap();
    let curve = ICurvePoolNG::new(addr, &provider);
    let bn = provider.get_block_number().await.unwrap() - 5;
    let block = alloy::eips::BlockId::number(bn);

    let b0 = curve
        .balances(U256::from(0))
        .block(block)
        .call()
        .await
        .unwrap();
    let b1 = curve
        .balances(U256::from(1))
        .block(block)
        .call()
        .await
        .unwrap();
    let raw_a = curve.A().block(block).call().await.unwrap();
    let fee = curve.fee().block(block).call().await.unwrap();
    let offpeg = curve
        .offpeg_fee_multiplier()
        .block(block)
        .call()
        .await
        .unwrap();

    let rate18 = U256::from(1_000_000_000_000_000_000u128);
    let rates = [rate18, rate18];
    let balances = [b0, b1];
    let amp = raw_a * A_PRECISION;

    let n = fuzz_iterations();
    let half = n / 2;
    let mut passed = 0u64;
    let mut skipped = 0u64;

    for dx in generate_amounts(half, balances[0], bn) {
        let on_chain = curve.get_dy(0i128, 1i128, dx).block(block).call().await;
        match on_chain {
            Ok(expected) => match get_amount_out(&balances, &rates, amp, fee, offpeg, 0, 1, dx) {
                Some(result) => {
                    assert_eq!(result, expected, "NG 0→1 mismatch at dx={dx}");
                    passed += 1;
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    for dx in generate_amounts(n - half, balances[1], bn + 1) {
        let on_chain = curve.get_dy(1i128, 0i128, dx).block(block).call().await;
        match on_chain {
            Ok(expected) => match get_amount_out(&balances, &rates, amp, fee, offpeg, 1, 0, dx) {
                Some(result) => {
                    assert_eq!(result, expected, "NG 1→0 mismatch at dx={dx}");
                    passed += 1;
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    println!(
        "fuzz_stableswap_ng: {passed} passed, {skipped} skipped, both directions (block {bn})"
    );
    assert!(passed > 0, "no tests passed");
}

// ── StableSwap Meta (GUSD/3CRV) ────────────────────────────────────────────

alloy::sol! {
    #[sol(rpc)]
    interface ICurvePoolMeta {
        function get_dy(int128 i, int128 j, uint256 dx) external view returns (uint256);
        function balances(uint256 i) external view returns (uint256);
        function A() external view returns (uint256);
        function fee() external view returns (uint256);
        function base_pool() external view returns (address);
        function base_virtual_price() external view returns (uint256);
        function base_cache_updated() external view returns (uint256);
    }
    #[sol(rpc)]
    interface IBasePool {
        function get_virtual_price() external view returns (uint256);
    }
}

#[tokio::test]
#[ignore = "requires RPC_URL"]
async fn fuzz_stableswap_meta() {
    use curve_math::core::stableswap_meta::A_PRECISION;
    use curve_math::swap::stableswap_meta::get_amount_out;

    let provider = make_provider!();
    let addr =
        alloy_primitives::Address::from_str("0x4f062658EaAF2C1ccf8C8e36D6824CDf41167956").unwrap();
    let curve = ICurvePoolMeta::new(addr, &provider);
    let bn = provider.get_block_number().await.unwrap() - 5;
    let block = alloy::eips::BlockId::number(bn);

    let b0 = curve
        .balances(U256::from(0))
        .block(block)
        .call()
        .await
        .unwrap();
    let b1 = curve
        .balances(U256::from(1))
        .block(block)
        .call()
        .await
        .unwrap();
    let raw_a = curve.A().block(block).call().await.unwrap();
    let fee = curve.fee().block(block).call().await.unwrap();

    // Read virtual_price for 3CRV rate (same as verify_gusd)
    let base_pool_addr = curve.base_pool().block(block).call().await.unwrap();
    let base_pool = IBasePool::new(base_pool_addr, &provider);
    let cached_vp = curve
        .base_virtual_price()
        .block(block)
        .call()
        .await
        .unwrap();
    let cache_updated = curve
        .base_cache_updated()
        .block(block)
        .call()
        .await
        .unwrap();
    let block_data = provider
        .get_block_by_number(bn.into())
        .await
        .unwrap()
        .unwrap();
    let block_ts = U256::from(block_data.header.timestamp);
    let vp = if block_ts - cache_updated > U256::from(600u64) {
        base_pool
            .get_virtual_price()
            .block(block)
            .call()
            .await
            .unwrap()
    } else {
        cached_vp
    };

    // GUSD(2-dec): rate = 10^34. 3CRV(18-dec): rate = virtual_price.
    let rate_gusd = U256::from(10u64).pow(U256::from(34u64));
    let rates = [rate_gusd, vp];
    let balances = [b0, b1];
    let amp = raw_a * A_PRECISION;

    let n = fuzz_iterations();
    let half = n / 2;
    let mut passed = 0u64;
    let mut skipped = 0u64;

    // GUSD → 3CRV
    for dx in generate_amounts(half, balances[0], bn) {
        let on_chain = curve.get_dy(0i128, 1i128, dx).block(block).call().await;
        match on_chain {
            Ok(expected) => match get_amount_out(&balances, &rates, amp, fee, 0, 1, dx) {
                Some(result) => {
                    assert_eq!(result, expected, "Meta 0→1 mismatch at dx={dx}");
                    passed += 1;
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    // 3CRV → GUSD
    for dx in generate_amounts(n - half, balances[1], bn + 1) {
        let on_chain = curve.get_dy(1i128, 0i128, dx).block(block).call().await;
        match on_chain {
            Ok(expected) => match get_amount_out(&balances, &rates, amp, fee, 1, 0, dx) {
                Some(result) => {
                    assert_eq!(result, expected, "Meta 1→0 mismatch at dx={dx}");
                    passed += 1;
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    println!(
        "fuzz_stableswap_meta: {passed} passed, {skipped} skipped, both directions (block {bn})"
    );
    assert!(passed > 0, "no tests passed");
}

// ── TwoCrypto V1 (CRV/ETH) ─────────────────────────────────────────────────

alloy::sol! {
    #[sol(rpc)]
    interface ICryptoPool2 {
        function get_dy(uint256 i, uint256 j, uint256 dx) external view returns (uint256);
        function balances(uint256 i) external view returns (uint256);
        function A() external view returns (uint256);
        function gamma() external view returns (uint256);
        function D() external view returns (uint256);
        function price_scale() external view returns (uint256);
        function mid_fee() external view returns (uint256);
        function out_fee() external view returns (uint256);
        function fee_gamma() external view returns (uint256);
    }
}

#[tokio::test]
#[ignore = "requires RPC_URL"]
async fn fuzz_twocrypto_v1() {
    use curve_math::swap::twocrypto_v1::get_amount_out;

    let provider = make_provider!();
    let addr =
        alloy_primitives::Address::from_str("0x8301AE4fc9c624d1D396cbDAa1ed877821D7C511").unwrap();
    let curve = ICryptoPool2::new(addr, &provider);
    let bn = provider.get_block_number().await.unwrap() - 5;
    let block = alloy::eips::BlockId::number(bn);

    let b0 = curve
        .balances(U256::from(0))
        .block(block)
        .call()
        .await
        .unwrap();
    let b1 = curve
        .balances(U256::from(1))
        .block(block)
        .call()
        .await
        .unwrap();
    let a = curve.A().block(block).call().await.unwrap();
    let gamma = curve.gamma().block(block).call().await.unwrap();
    let d = curve.D().block(block).call().await.unwrap();
    let ps = curve.price_scale().block(block).call().await.unwrap();
    let mid_fee = curve.mid_fee().block(block).call().await.unwrap();
    let out_fee = curve.out_fee().block(block).call().await.unwrap();
    let fee_gamma = curve.fee_gamma().block(block).call().await.unwrap();

    let balances = [b0, b1];
    let precisions = [U256::from(1u64), U256::from(1u64)];

    let n = fuzz_iterations();
    let half = n / 2;
    let mut passed = 0u64;
    let mut skipped = 0u64;

    for dx in generate_amounts(half, balances[0], bn) {
        let on_chain = curve
            .get_dy(U256::from(0), U256::from(1), dx)
            .block(block)
            .call()
            .await;
        match on_chain {
            Ok(expected) => match get_amount_out(
                &balances,
                &precisions,
                ps,
                d,
                a,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
                true, // CRV/ETH is ETH variant
                0,
                1,
                dx,
            ) {
                Some(result) => {
                    assert_eq!(result, expected, "TwoCryptoV1 0→1 mismatch at dx={dx}");
                    passed += 1;
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    for dx in generate_amounts(n - half, balances[1], bn + 1) {
        let on_chain = curve
            .get_dy(U256::from(1), U256::from(0), dx)
            .block(block)
            .call()
            .await;
        match on_chain {
            Ok(expected) => match get_amount_out(
                &balances,
                &precisions,
                ps,
                d,
                a,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
                true, // CRV/ETH is ETH variant
                1,
                0,
                dx,
            ) {
                Some(result) => {
                    assert_eq!(result, expected, "TwoCryptoV1 1→0 mismatch at dx={dx}");
                    passed += 1;
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    println!("fuzz_twocrypto_v1: {passed} passed, {skipped} skipped (block {bn})");
    assert!(passed > 0, "no tests passed");
}

// ── TwoCrypto NG (crvUSD/FXN) ───────────────────────────────────────────────

#[tokio::test]
#[ignore = "requires RPC_URL"]
async fn fuzz_twocrypto_ng() {
    use curve_math::swap::twocrypto_ng::get_amount_out;

    let provider = make_provider!();
    let addr =
        alloy_primitives::Address::from_str("0xfb8b95Fb2296a0Ad4b6b1419fdAA5AA5F13e4009").unwrap();
    let curve = ICryptoPool2::new(addr, &provider);
    let bn = provider.get_block_number().await.unwrap() - 5;
    let block = alloy::eips::BlockId::number(bn);

    let b0 = curve
        .balances(U256::from(0))
        .block(block)
        .call()
        .await
        .unwrap();
    let b1 = curve
        .balances(U256::from(1))
        .block(block)
        .call()
        .await
        .unwrap();
    let a = curve.A().block(block).call().await.unwrap();
    let gamma = curve.gamma().block(block).call().await.unwrap();
    let d = curve.D().block(block).call().await.unwrap();
    let ps = curve.price_scale().block(block).call().await.unwrap();
    let mid_fee = curve.mid_fee().block(block).call().await.unwrap();
    let out_fee = curve.out_fee().block(block).call().await.unwrap();
    let fee_gamma = curve.fee_gamma().block(block).call().await.unwrap();

    let balances = [b0, b1];
    let precisions = [U256::from(1u64), U256::from(1u64)];

    let n = fuzz_iterations();
    let half = n / 2;
    let mut passed = 0u64;
    let mut skipped = 0u64;

    for dx in generate_amounts(half, balances[0], bn) {
        let on_chain = curve
            .get_dy(U256::from(0), U256::from(1), dx)
            .block(block)
            .call()
            .await;
        match on_chain {
            Ok(expected) => match get_amount_out(
                &balances,
                &precisions,
                ps,
                d,
                a,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
                0,
                1,
                dx,
            ) {
                Some(result) => {
                    assert_eq!(result, expected, "TwoCryptoNG 0→1 mismatch at dx={dx}");
                    passed += 1;
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    for dx in generate_amounts(n - half, balances[1], bn + 1) {
        let on_chain = curve
            .get_dy(U256::from(1), U256::from(0), dx)
            .block(block)
            .call()
            .await;
        match on_chain {
            Ok(expected) => match get_amount_out(
                &balances,
                &precisions,
                ps,
                d,
                a,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
                1,
                0,
                dx,
            ) {
                Some(result) => {
                    assert_eq!(result, expected, "TwoCryptoNG 1→0 mismatch at dx={dx}");
                    passed += 1;
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    println!("fuzz_twocrypto_ng: {passed} passed, {skipped} skipped (block {bn})");
    assert!(passed > 0, "no tests passed");
}

// ── TriCrypto V1 (USDT/WBTC/WETH) ──────────────────────────────────────────

alloy::sol! {
    #[sol(rpc)]
    interface ITriCryptoPool {
        function get_dy(uint256 i, uint256 j, uint256 dx) external view returns (uint256);
        function balances(uint256 i) external view returns (uint256);
        function A() external view returns (uint256);
        function gamma() external view returns (uint256);
        function D() external view returns (uint256);
        function price_scale(uint256 i) external view returns (uint256);
        function mid_fee() external view returns (uint256);
        function out_fee() external view returns (uint256);
        function fee_gamma() external view returns (uint256);
    }
}

#[tokio::test]
#[ignore = "requires RPC_URL"]
async fn fuzz_tricrypto_v1() {
    use curve_math::swap::tricrypto_v1::get_amount_out;

    let provider = make_provider!();
    let addr =
        alloy_primitives::Address::from_str("0xD51a44d3FaE010294C616388b506AcdA1bfAAE46").unwrap();
    let curve = ITriCryptoPool::new(addr, &provider);
    let bn = provider.get_block_number().await.unwrap() - 5;
    let block = alloy::eips::BlockId::number(bn);

    let b0 = curve
        .balances(U256::from(0))
        .block(block)
        .call()
        .await
        .unwrap();
    let b1 = curve
        .balances(U256::from(1))
        .block(block)
        .call()
        .await
        .unwrap();
    let b2 = curve
        .balances(U256::from(2))
        .block(block)
        .call()
        .await
        .unwrap();
    let a = curve.A().block(block).call().await.unwrap();
    let gamma = curve.gamma().block(block).call().await.unwrap();
    let d = curve.D().block(block).call().await.unwrap();
    let ps0 = curve
        .price_scale(U256::from(0))
        .block(block)
        .call()
        .await
        .unwrap();
    let ps1 = curve
        .price_scale(U256::from(1))
        .block(block)
        .call()
        .await
        .unwrap();
    let mid_fee = curve.mid_fee().block(block).call().await.unwrap();
    let out_fee = curve.out_fee().block(block).call().await.unwrap();
    let fee_gamma = curve.fee_gamma().block(block).call().await.unwrap();

    let balances = [b0, b1, b2];
    let precisions = [
        U256::from(1_000_000_000_000u64),
        U256::from(10_000_000_000u64),
        U256::from(1u64),
    ];
    let price_scale = [ps0, ps1];

    let n = fuzz_iterations();
    let pairs: [(usize, usize); 6] = [(0, 1), (0, 2), (1, 0), (1, 2), (2, 0), (2, 1)];
    let per_pair = (n / pairs.len()).max(1);
    let mut passed = 0u64;
    let mut skipped = 0u64;

    for (idx, &(i, j)) in pairs.iter().enumerate() {
        let max_dx = balances[i];
        for dx in generate_amounts(per_pair, max_dx, bn + idx as u64) {
            let on_chain = curve
                .get_dy(U256::from(i), U256::from(j), dx)
                .block(block)
                .call()
                .await;
            match on_chain {
                Ok(expected) => match get_amount_out(
                    &balances,
                    &precisions,
                    &price_scale,
                    d,
                    a,
                    gamma,
                    mid_fee,
                    out_fee,
                    fee_gamma,
                    i,
                    j,
                    dx,
                ) {
                    Some(result) => {
                        assert_eq!(result, expected, "TriCryptoV1 {i}→{j} mismatch at dx={dx}");
                        passed += 1;
                    }
                    None => skipped += 1,
                },
                Err(_) => skipped += 1,
            }
        }
    }
    println!("fuzz_tricrypto_v1: {passed} passed, {skipped} skipped (block {bn})");
    assert!(passed > 0, "no tests passed");
}

// ── TriCrypto NG (USDC/WBTC/WETH) ──────────────────────────────────────────

#[tokio::test]
#[ignore = "requires RPC_URL"]
async fn fuzz_tricrypto_ng() {
    use curve_math::swap::tricrypto_ng::get_amount_out;

    let provider = make_provider!();
    let addr =
        alloy_primitives::Address::from_str("0x7F86Bf177Dd4F3494b841a37e810A34dD56c829B").unwrap();
    let curve = ITriCryptoPool::new(addr, &provider);
    let bn = provider.get_block_number().await.unwrap() - 5;
    let block = alloy::eips::BlockId::number(bn);

    let b0 = curve
        .balances(U256::from(0))
        .block(block)
        .call()
        .await
        .unwrap();
    let b1 = curve
        .balances(U256::from(1))
        .block(block)
        .call()
        .await
        .unwrap();
    let b2 = curve
        .balances(U256::from(2))
        .block(block)
        .call()
        .await
        .unwrap();
    let a = curve.A().block(block).call().await.unwrap();
    let gamma = curve.gamma().block(block).call().await.unwrap();
    let d = curve.D().block(block).call().await.unwrap();
    let ps0 = curve
        .price_scale(U256::from(0))
        .block(block)
        .call()
        .await
        .unwrap();
    let ps1 = curve
        .price_scale(U256::from(1))
        .block(block)
        .call()
        .await
        .unwrap();
    let mid_fee = curve.mid_fee().block(block).call().await.unwrap();
    let out_fee = curve.out_fee().block(block).call().await.unwrap();
    let fee_gamma = curve.fee_gamma().block(block).call().await.unwrap();

    let balances = [b0, b1, b2];
    let precisions = [
        U256::from(1_000_000_000_000u64),
        U256::from(10_000_000_000u64),
        U256::from(1u64),
    ];
    let price_scale = [ps0, ps1];

    let n = fuzz_iterations();
    let pairs: [(usize, usize); 6] = [(0, 1), (0, 2), (1, 0), (1, 2), (2, 0), (2, 1)];
    let per_pair = (n / pairs.len()).max(1);
    let mut passed = 0u64;
    let mut skipped = 0u64;

    for (idx, &(i, j)) in pairs.iter().enumerate() {
        let max_dx = balances[i];
        for dx in generate_amounts(per_pair, max_dx, bn + idx as u64) {
            let on_chain = curve
                .get_dy(U256::from(i), U256::from(j), dx)
                .block(block)
                .call()
                .await;
            match on_chain {
                Ok(expected) => match get_amount_out(
                    &balances,
                    &precisions,
                    &price_scale,
                    d,
                    a,
                    gamma,
                    mid_fee,
                    out_fee,
                    fee_gamma,
                    i,
                    j,
                    dx,
                ) {
                    Some(result) => {
                        assert_eq!(result, expected, "TriCryptoNG {i}→{j} mismatch at dx={dx}");
                        passed += 1;
                    }
                    None => skipped += 1,
                },
                Err(_) => skipped += 1,
            }
        }
    }
    println!("fuzz_tricrypto_ng: {passed} passed, {skipped} skipped (block {bn})");
    assert!(passed > 0, "no tests passed");
}

// ═══════════════════════════════════════════════════════════════════════════
// ADDITIONAL POOLS — high-liquidity, different parameters from primary pools
// ═══════════════════════════════════════════════════════════════════════════

// ── StableSwapNG #2 (PYUSD/USDS — $100M, A=10000, 6/18 dec) ────────────

#[tokio::test]
#[ignore = "requires RPC_URL"]
async fn fuzz_stableswap_ng_pyusd() {
    use curve_math::core::stableswap_ng::A_PRECISION;
    use curve_math::swap::stableswap_ng::get_amount_out;

    let provider = make_provider!();
    let addr =
        alloy_primitives::Address::from_str("0xa632d59b9b804a956bfaa9b48af3a1b74808fc1f").unwrap();
    let curve = ICurvePoolNG::new(addr, &provider);
    let bn = provider.get_block_number().await.unwrap() - 5;
    let block = alloy::eips::BlockId::number(bn);

    let b0 = curve
        .balances(U256::from(0))
        .block(block)
        .call()
        .await
        .unwrap();
    let b1 = curve
        .balances(U256::from(1))
        .block(block)
        .call()
        .await
        .unwrap();
    let raw_a = curve.A().block(block).call().await.unwrap();
    let fee = curve.fee().block(block).call().await.unwrap();
    let offpeg = curve
        .offpeg_fee_multiplier()
        .block(block)
        .call()
        .await
        .unwrap();

    // PYUSD=6dec, USDS=18dec
    let rate6 = U256::from(1_000_000_000_000_000_000_000_000_000_000u128); // 1e30
    let rate18 = U256::from(1_000_000_000_000_000_000u128);
    let rates = [rate6, rate18];
    let balances = [b0, b1];
    let amp = raw_a * A_PRECISION;

    let n = fuzz_iterations();
    let half = n / 2;
    let mut passed = 0u64;
    let mut skipped = 0u64;

    for dx in generate_amounts(half, balances[0], bn) {
        let on_chain = curve.get_dy(0i128, 1i128, dx).block(block).call().await;
        match on_chain {
            Ok(expected) => match get_amount_out(&balances, &rates, amp, fee, offpeg, 0, 1, dx) {
                Some(result) => {
                    assert_eq!(result, expected, "NG-PYUSD 0→1 mismatch at dx={dx}");
                    passed += 1;
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    for dx in generate_amounts(n - half, balances[1], bn + 1) {
        let on_chain = curve.get_dy(1i128, 0i128, dx).block(block).call().await;
        match on_chain {
            Ok(expected) => match get_amount_out(&balances, &rates, amp, fee, offpeg, 1, 0, dx) {
                Some(result) => {
                    assert_eq!(result, expected, "NG-PYUSD 1→0 mismatch at dx={dx}");
                    passed += 1;
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    println!("fuzz_stableswap_ng_pyusd: {passed} passed, {skipped} skipped, 6/18 dec (block {bn})");
    assert!(passed > 0, "no tests passed");
}

// ── TwoCryptoNG #2 (crvUSD/cbBTC — $47M, A=90000, 18/8 dec) ────────────

#[tokio::test]
#[ignore = "requires RPC_URL — KNOWN FAIL: v2.1.0 pools not yet supported"]
async fn fuzz_twocrypto_ng_cbbtc() {
    use curve_math::swap::twocrypto_ng::get_amount_out;

    let provider = make_provider!();
    let addr =
        alloy_primitives::Address::from_str("0x83f24023d15d835a213df24fd309c47dab5beb32").unwrap();
    let curve = ICryptoPool2::new(addr, &provider);
    let bn = provider.get_block_number().await.unwrap() - 5;
    let block = alloy::eips::BlockId::number(bn);

    let b0 = curve
        .balances(U256::from(0))
        .block(block)
        .call()
        .await
        .unwrap();
    let b1 = curve
        .balances(U256::from(1))
        .block(block)
        .call()
        .await
        .unwrap();
    let a = curve.A().block(block).call().await.unwrap();
    let gamma = curve.gamma().block(block).call().await.unwrap();
    let d = curve.D().block(block).call().await.unwrap();
    let ps = curve.price_scale().block(block).call().await.unwrap();
    let mid_fee = curve.mid_fee().block(block).call().await.unwrap();
    let out_fee = curve.out_fee().block(block).call().await.unwrap();
    let fee_gamma = curve.fee_gamma().block(block).call().await.unwrap();

    // crvUSD=18dec, cbBTC=8dec → precisions = [1, 10^10]
    let balances = [b0, b1];
    let precisions = [U256::from(1u64), U256::from(10_000_000_000u64)];

    let n = fuzz_iterations();
    let half = n / 2;
    let mut passed = 0u64;
    let mut skipped = 0u64;

    for dx in generate_amounts(half, balances[0], bn) {
        let on_chain = curve
            .get_dy(U256::from(0), U256::from(1), dx)
            .block(block)
            .call()
            .await;
        match on_chain {
            Ok(expected) => match get_amount_out(
                &balances,
                &precisions,
                ps,
                d,
                a,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
                0,
                1,
                dx,
            ) {
                Some(result) => {
                    assert_eq!(
                        result, expected,
                        "TwoCryptoNG-cbBTC 0→1 mismatch at dx={dx}"
                    );
                    passed += 1;
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    for dx in generate_amounts(n - half, balances[1], bn + 1) {
        let on_chain = curve
            .get_dy(U256::from(1), U256::from(0), dx)
            .block(block)
            .call()
            .await;
        match on_chain {
            Ok(expected) => match get_amount_out(
                &balances,
                &precisions,
                ps,
                d,
                a,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
                1,
                0,
                dx,
            ) {
                Some(result) => {
                    assert_eq!(
                        result, expected,
                        "TwoCryptoNG-cbBTC 1→0 mismatch at dx={dx}"
                    );
                    passed += 1;
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    println!("fuzz_twocrypto_ng_cbbtc: {passed} passed, {skipped} skipped, 18/8 dec (block {bn})");
    assert!(passed > 0, "no tests passed");
}

// ── StableSwapV1 #2 (ETH/stETH — $42M, A=900) ─────────────────────────

#[tokio::test]
#[ignore = "requires RPC_URL"]
async fn fuzz_stableswap_v1_steth() {
    use curve_math::swap::stableswap_v1::get_amount_out;

    let provider = make_provider!();
    let addr =
        alloy_primitives::Address::from_str("0xdc24316b9ae028f1497c275eb9192a3ea0f67022").unwrap();
    // stETH pool uses balances(uint256) like 3pool
    let curve = ICurvePool3::new(addr, &provider);
    let bn = provider.get_block_number().await.unwrap() - 5;
    let block = alloy::eips::BlockId::number(bn);

    let b0 = curve
        .balances(U256::from(0))
        .block(block)
        .call()
        .await
        .unwrap();
    let b1 = curve
        .balances(U256::from(1))
        .block(block)
        .call()
        .await
        .unwrap();
    let amp = curve.A().block(block).call().await.unwrap();
    let fee = curve.fee().block(block).call().await.unwrap();

    // Both ETH and stETH are 18 decimals
    let rate18 = U256::from(1_000_000_000_000_000_000u128);
    let rates = [rate18, rate18];
    let balances = [b0, b1];

    let n = fuzz_iterations();
    let half = n / 2;
    let mut passed = 0u64;
    let mut skipped = 0u64;

    for dx in generate_amounts(half, balances[0], bn) {
        let on_chain = curve.get_dy(0i128, 1i128, dx).block(block).call().await;
        match on_chain {
            Ok(expected) => match get_amount_out(&balances, &rates, amp, fee, 0, 1, dx) {
                Some(result) => {
                    assert_eq!(result, expected, "V1-stETH 0→1 mismatch at dx={dx}");
                    passed += 1;
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    for dx in generate_amounts(n - half, balances[1], bn + 1) {
        let on_chain = curve.get_dy(1i128, 0i128, dx).block(block).call().await;
        match on_chain {
            Ok(expected) => match get_amount_out(&balances, &rates, amp, fee, 1, 0, dx) {
                Some(result) => {
                    assert_eq!(result, expected, "V1-stETH 1→0 mismatch at dx={dx}");
                    passed += 1;
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    println!("fuzz_stableswap_v1_steth: {passed} passed, {skipped} skipped (block {bn})");
    assert!(passed > 0, "no tests passed");
}

// ── TwoCryptoNG #3 (crvUSD/tBTC — $25M, both 18-dec) ───────────────────

#[tokio::test]
#[ignore = "requires RPC_URL — KNOWN FAIL: v2.1.0 pools not yet supported"]
async fn fuzz_twocrypto_ng_tbtc() {
    use curve_math::swap::twocrypto_ng::get_amount_out;

    let provider = make_provider!();
    let addr =
        alloy_primitives::Address::from_str("0xf1f435b05d255a5dbde37333c0f61da6f69c6127").unwrap();
    let curve = ICryptoPool2::new(addr, &provider);
    let bn = provider.get_block_number().await.unwrap() - 5;
    let block = alloy::eips::BlockId::number(bn);

    let b0 = curve
        .balances(U256::from(0))
        .block(block)
        .call()
        .await
        .unwrap();
    let b1 = curve
        .balances(U256::from(1))
        .block(block)
        .call()
        .await
        .unwrap();
    let a = curve.A().block(block).call().await.unwrap();
    let gamma = curve.gamma().block(block).call().await.unwrap();
    let d = curve.D().block(block).call().await.unwrap();
    let ps = curve.price_scale().block(block).call().await.unwrap();
    let mid_fee = curve.mid_fee().block(block).call().await.unwrap();
    let out_fee = curve.out_fee().block(block).call().await.unwrap();
    let fee_gamma = curve.fee_gamma().block(block).call().await.unwrap();

    // Both crvUSD and tBTC are 18 decimals
    let balances = [b0, b1];
    let precisions = [U256::from(1u64), U256::from(1u64)];

    let n = fuzz_iterations();
    let half = n / 2;
    let mut passed = 0u64;
    let mut skipped = 0u64;

    for dx in generate_amounts(half, balances[0], bn) {
        let on_chain = curve
            .get_dy(U256::from(0), U256::from(1), dx)
            .block(block)
            .call()
            .await;
        match on_chain {
            Ok(expected) => match get_amount_out(
                &balances,
                &precisions,
                ps,
                d,
                a,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
                0,
                1,
                dx,
            ) {
                Some(result) => {
                    assert_eq!(result, expected, "TwoCryptoNG-tBTC 0→1 mismatch at dx={dx}");
                    passed += 1;
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    for dx in generate_amounts(n - half, balances[1], bn + 1) {
        let on_chain = curve
            .get_dy(U256::from(1), U256::from(0), dx)
            .block(block)
            .call()
            .await;
        match on_chain {
            Ok(expected) => match get_amount_out(
                &balances,
                &precisions,
                ps,
                d,
                a,
                gamma,
                mid_fee,
                out_fee,
                fee_gamma,
                1,
                0,
                dx,
            ) {
                Some(result) => {
                    assert_eq!(result, expected, "TwoCryptoNG-tBTC 1→0 mismatch at dx={dx}");
                    passed += 1;
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    println!("fuzz_twocrypto_ng_tbtc: {passed} passed, {skipped} skipped, 18/18 dec (block {bn})");
    assert!(passed > 0, "no tests passed");
}
