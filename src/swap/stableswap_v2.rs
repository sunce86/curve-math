//! Pool-level get_amount_out for StableSwapV2 (FRAX/USDC, stETH, factory plain).
//!
//! -1 offset. Fee before denormalize.

use alloy_primitives::U256;

use crate::core::stableswap_v2::{get_d, get_y, A_PRECISION, FEE_DENOMINATOR, PRECISION};

pub fn get_amount_out(
    balances: &[U256],
    rates: &[U256],
    amp: U256,
    fee: U256,
    i: usize,
    j: usize,
    dx: U256,
) -> Option<U256> {
    if dx.is_zero() {
        return None;
    }

    let precision = U256::from(PRECISION);

    let xp: Vec<U256> = balances
        .iter()
        .zip(rates.iter())
        .map(|(b, r)| *b * *r / precision)
        .collect();

    let d = get_d(&xp, amp)?;
    let x_new = xp[i] + dx * rates[i] / precision;
    let y_new = get_y(i, j, x_new, &xp, d, amp)?;

    if xp[j] <= y_new {
        return None;
    }

    let dy = xp[j] - y_new - U256::from(1);
    let fee_amount = fee * dy / U256::from(FEE_DENOMINATOR);
    let result = (dy - fee_amount) * precision / rates[j];

    if result.is_zero() {
        return None;
    }

    Some(result)
}

pub fn get_amount_in(
    balances: &[U256],
    rates: &[U256],
    amp: U256,
    fee: U256,
    i: usize,
    j: usize,
    desired_output: U256,
) -> Option<U256> {
    if desired_output.is_zero() {
        return None;
    }
    let precision = U256::from(PRECISION);
    let fee_denom = U256::from(FEE_DENOMINATOR);
    let xp: Vec<U256> = balances
        .iter()
        .zip(rates.iter())
        .map(|(b, r)| *b * *r / precision)
        .collect();
    let d = get_d(&xp, amp)?;
    // Reverse denorm: dy_after_fee_internal = desired_output * rates[j] / PRECISION
    let dy_after_fee_internal = desired_output * rates[j] / precision;
    // Reverse fee (fee was applied in internal space)
    let complement = fee_denom - fee;
    let dy_internal = (dy_after_fee_internal * fee_denom + complement - U256::from(1)) / complement;
    if xp[j] <= dy_internal + U256::from(1) {
        return None;
    }
    // -1 offset
    let y_new = xp[j] - dy_internal - U256::from(1);
    let x_new = get_y(j, i, y_new, &xp, d, amp)?;
    if x_new <= xp[i] {
        return None;
    }
    let dx = (x_new - xp[i]) * precision / rates[i] + U256::from(1);
    Some(dx)
}

/// Spot price dy/dx including fee, returned as (numerator, denominator).
/// Analytical: from implicit differentiation of StableSwap invariant.
pub fn spot_price(
    balances: &[U256],
    rates: &[U256],
    amp: U256,
    fee: U256,
    i: usize,
    j: usize,
) -> Option<(U256, U256)> {
    let precision = U256::from(PRECISION);
    let fee_denom = U256::from(FEE_DENOMINATOR);
    let n = U256::from(balances.len());
    let ann_eff = amp.checked_mul(n)? / U256::from(A_PRECISION);
    let xp: Vec<U256> = balances
        .iter()
        .zip(rates.iter())
        .map(|(b, r)| *b * *r / precision)
        .collect();
    let d = get_d(&xp, amp)?;
    // D_P = D^(N+1) / (N^N * prod(xp)), computed iteratively
    let mut d_p = d;
    for x_k in &xp {
        d_p = d_p.checked_mul(d)?.checked_div(x_k.checked_mul(n)?)?;
    }
    // raw_price(i->j) = (ann_eff * xp[i] - d_p) / (ann_eff * xp[j] - d_p)
    let num_xp = ann_eff.checked_mul(xp[i])?.checked_sub(d_p)?;
    let den_xp = ann_eff.checked_mul(xp[j])?.checked_sub(d_p)?;
    if num_xp.is_zero() || den_xp.is_zero() {
        return None;
    }
    // Convert to native units: multiply by rates[i]/rates[j]
    // Include fee: multiply numerator by (FEE_DENOM - fee)
    let numerator = num_xp * rates[i] * (fee_denom - fee);
    let denominator = den_xp * rates[j] * fee_denom;
    Some((numerator, denominator))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::stableswap_v2::A_PRECISION;

    const RATE_18: u128 = 1_000_000_000_000_000_000;
    const RATE_6: u128 = 1_000_000_000_000_000_000_000_000_000_000;

    #[test]
    fn basic_swap() {
        let b = U256::from(1_000_000_000_000_000_000_000_000u128);
        let out = get_amount_out(
            &[b, b],
            &[U256::from(RATE_18), U256::from(RATE_18)],
            U256::from(200u64 * A_PRECISION as u64),
            U256::from(4_000_000u64),
            0,
            1,
            U256::from(1_000_000_000_000_000_000_000u128),
        )
        .expect("swap");

        let amount_in = U256::from(1_000_000_000_000_000_000_000u128);
        assert!(out < amount_in);
        assert!(out > amount_in * U256::from(999) / U256::from(1000));
    }

    #[test]
    fn roundtrip() {
        let b = U256::from(1_000_000_000_000_000_000_000_000u128);
        let balances = [b, b];
        let rates = [U256::from(RATE_18), U256::from(RATE_18)];
        let amp = U256::from(200u64 * A_PRECISION as u64);
        let fee = U256::from(4_000_000u64);
        let dx = U256::from(1_000_000_000_000_000_000_000u128);
        let dy = get_amount_out(&balances, &rates, amp, fee, 0, 1, dx).expect("out");
        let dx_recovered = get_amount_in(&balances, &rates, amp, fee, 0, 1, dy).expect("in");
        assert!(dx_recovered >= dx);
        assert!(dx_recovered <= dx + U256::from(2));
        let dy_check =
            get_amount_out(&balances, &rates, amp, fee, 0, 1, dx_recovered).expect("check");
        assert!(dy_check >= dy);
    }

    #[test]
    fn roundtrip_different_decimals() {
        let balances = [
            U256::from(1_000_000_000_000_000_000_000_000u128),
            U256::from(1_000_000_000_000u128),
        ];
        let rates = [U256::from(RATE_18), U256::from(RATE_6)];
        let amp = U256::from(200u64 * A_PRECISION as u64);
        let fee = U256::from(4_000_000u64);
        let dx = U256::from(1_000_000_000u128); // 1k USDC
        let dy = get_amount_out(&balances, &rates, amp, fee, 1, 0, dx).expect("out");
        let dx_recovered = get_amount_in(&balances, &rates, amp, fee, 1, 0, dy).expect("in");
        assert!(dx_recovered >= dx);
        assert!(dx_recovered <= dx + U256::from(2));
        let dy_check =
            get_amount_out(&balances, &rates, amp, fee, 1, 0, dx_recovered).expect("check");
        assert!(dy_check >= dy);
    }

    #[test]
    fn zero_returns_none() {
        let b = U256::from(1_000_000_000_000_000_000_000_000u128);
        assert!(get_amount_out(
            &[b, b],
            &[U256::from(RATE_18), U256::from(RATE_18)],
            U256::from(200u64 * A_PRECISION as u64),
            U256::from(4_000_000u64),
            0,
            1,
            U256::ZERO,
        )
        .is_none());
    }

    #[test]
    fn spot_price_balanced_near_one() {
        let b = U256::from(1_000_000_000_000_000_000_000_000u128);
        let balances = [b, b];
        let rates = [U256::from(RATE_18), U256::from(RATE_18)];
        let amp = U256::from(200u64 * A_PRECISION as u64);
        let fee = U256::from(4_000_000u64);
        let (num, den) = spot_price(&balances, &rates, amp, fee, 0, 1).expect("price");
        let diff = if num > den { num - den } else { den - num };
        assert!(diff * U256::from(1000) < den, "spot price not near 1");
    }

    #[test]
    fn spot_price_symmetry() {
        let b = U256::from(1_000_000_000_000_000_000_000_000u128);
        let balances = [b, b];
        let rates = [U256::from(RATE_18), U256::from(RATE_18)];
        let amp = U256::from(200u64 * A_PRECISION as u64);
        let fee = U256::from(4_000_000u64);
        let (num_ij, den_ij) = spot_price(&balances, &rates, amp, fee, 0, 1).expect("price_ij");
        let (num_ji, den_ji) = spot_price(&balances, &rates, amp, fee, 1, 0).expect("price_ji");
        let product_num = num_ij * num_ji;
        let product_den = den_ij * den_ji;
        let diff = if product_num > product_den {
            product_num - product_den
        } else {
            product_den - product_num
        };
        assert!(diff * U256::from(1000) < product_den, "symmetry violated");
    }

    #[test]
    fn spot_price_consistent_with_swap() {
        let b = U256::from(1_000_000_000_000_000_000_000_000u128);
        let balances = [b, b];
        let rates = [U256::from(RATE_18), U256::from(RATE_18)];
        let amp = U256::from(200u64 * A_PRECISION as u64);
        let fee = U256::from(4_000_000u64);
        let dx = U256::from(1_000_000_000_000_000u128);
        let dy = get_amount_out(&balances, &rates, amp, fee, 0, 1, dx).expect("out");
        let (num, den) = spot_price(&balances, &rates, amp, fee, 0, 1).expect("price");
        let lhs = dy * den;
        let rhs = dx * num;
        let diff = if lhs > rhs { lhs - rhs } else { rhs - lhs };
        assert!(
            diff * U256::from(100) < rhs,
            "spot price inconsistent with swap"
        );
    }

    alloy::sol! {
        #[sol(rpc)]
        interface ICurvePool {
            function get_dy(int128 i, int128 j, uint256 dx) external view returns (uint256);
            function balances(uint256 i) external view returns (uint256);
            function A() external view returns (uint256);
            function fee() external view returns (uint256);
        }
    }

    #[tokio::test]
    #[ignore = "requires RPC_URL env var pointing to Ethereum mainnet"]
    async fn verify_frax_usdc() {
        use alloy::providers::{Provider, ProviderBuilder};
        use std::str::FromStr;

        let rpc_url = std::env::var("RPC_URL").expect("RPC_URL must be set");
        let provider =
            ProviderBuilder::new().connect_http(rpc_url.parse().expect("invalid RPC_URL"));

        let pool_address =
            alloy_primitives::Address::from_str("0xDcEF968d416a41Cdac0ED8702fAC8128A64241A2")
                .expect("valid");
        let curve = ICurvePool::new(pool_address, &provider);

        let block_num = provider.get_block_number().await.expect("block");
        let block = alloy::eips::BlockId::number(block_num);

        let r0 = curve
            .balances(U256::from(0))
            .block(block)
            .call()
            .await
            .expect("b0");
        let r1 = curve
            .balances(U256::from(1))
            .block(block)
            .call()
            .await
            .expect("b1");
        let raw_a = curve.A().block(block).call().await.expect("A");
        let pool_fee = curve.fee().block(block).call().await.expect("fee");

        let balances = [r0, r1];
        let rates = [U256::from(RATE_18), U256::from(RATE_6)];
        let amp = raw_a * U256::from(A_PRECISION as u64);

        for (amount, label) in [
            (U256::from(1_000_000_000_000_000_000u128), "1 FRAX"),
            (U256::from(1_000_000_000_000_000_000_000u128), "1k FRAX"),
            (U256::from(100_000_000_000_000_000_000_000u128), "100k FRAX"),
        ] {
            let on_chain = curve
                .get_dy(0i128, 1i128, amount)
                .block(block)
                .call()
                .await
                .expect("get_dy");
            let ours =
                get_amount_out(&balances, &rates, amp, pool_fee, 0, 1, amount).expect("ours");
            println!("  {label}: on_chain={on_chain}, ours={ours}");
            assert_eq!(ours, on_chain, "FRAX→USDC mismatch for {label}");
        }

        for (amount, label) in [
            (U256::from(1_000_000u128), "1 USDC"),
            (U256::from(1_000_000_000u128), "1k USDC"),
            (U256::from(100_000_000_000u128), "100k USDC"),
        ] {
            let on_chain = curve
                .get_dy(1i128, 0i128, amount)
                .block(block)
                .call()
                .await
                .expect("get_dy");
            let ours =
                get_amount_out(&balances, &rates, amp, pool_fee, 1, 0, amount).expect("ours");
            println!("  {label}: on_chain={on_chain}, ours={ours}");
            assert_eq!(ours, on_chain, "USDC→FRAX mismatch for {label}");
        }

        println!("StableSwapV2 (FRAX/USDC) verification passed!");
    }
}
