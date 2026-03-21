//! Pool-level swap functions for StableSwapV0 (sUSD, Compound, USDT, y, BUSD).
//!
//! NO -1 offset. Denorm FIRST, then fee.

use alloy_primitives::U256;

use crate::core::stableswap_v0::{get_d, get_y, A_PRECISION, FEE_DENOMINATOR, PRECISION};

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
    // NO -1 offset. Denorm FIRST, then fee.
    let dy = (xp[j] - y_new) * precision / rates[j];
    let fee_amount = fee * dy / U256::from(FEE_DENOMINATOR);
    let result = dy - fee_amount;
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
    // Reverse fee (round up to ensure sufficient input)
    let fee_complement = fee_denom - fee;
    let dy = (desired_output * fee_denom + fee_complement - U256::from(1)) / fee_complement;
    // Reverse denorm: dy_internal = dy * rates[j] / PRECISION
    let dy_internal = dy * rates[j] / precision;
    if xp[j] <= dy_internal {
        return None;
    }
    // NO -1 offset
    let y_new = xp[j] - dy_internal;
    // Solve for x_new using swapped indices
    let x_new = get_y(j, i, y_new, &xp, d, amp)?;
    if x_new <= xp[i] {
        return None;
    }
    // Denormalize input
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
    // raw_price(i→j) = (ann_eff * xp[i] - d_p) / (ann_eff * xp[j] - d_p)
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

    #[test]
    fn roundtrip() {
        let rate18 = U256::from(1_000_000_000_000_000_000u128);
        let balances = [
            U256::from(10_000_000_000_000_000_000_000_000u128),
            U256::from(10_000_000_000_000_000_000_000_000u128),
            U256::from(10_000_000_000_000_000_000_000_000u128),
            U256::from(10_000_000_000_000_000_000_000_000u128),
        ];
        let rates = [rate18, rate18, rate18, rate18];
        let amp = U256::from(100u64);
        let fee = U256::from(4_000_000u64);
        let dx = U256::from(1_000_000_000_000_000_000_000u128);
        let dy = get_amount_out(&balances, &rates, amp, fee, 0, 1, dx).expect("out");
        let dx_recovered = get_amount_in(&balances, &rates, amp, fee, 0, 1, dy).expect("in");
        assert!(dx_recovered >= dx);
        assert!(dx_recovered <= dx + U256::from(2));
        // Verify forward pass produces at least desired output
        let dy_check =
            get_amount_out(&balances, &rates, amp, fee, 0, 1, dx_recovered).expect("check");
        assert!(dy_check >= dy);
    }

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
    #[ignore = "requires RPC_URL env var pointing to Ethereum mainnet"]
    async fn verify_susd() {
        use alloy::providers::{Provider, ProviderBuilder};
        use std::str::FromStr;
        let rpc_url = std::env::var("RPC_URL").expect("RPC_URL");
        let provider = ProviderBuilder::new().connect_http(rpc_url.parse().expect("url"));
        let addr =
            alloy_primitives::Address::from_str("0xA5407eAE9Ba41422680e2e00537571bcC53efBfD")
                .expect("addr");
        let curve = ICurvePoolOld::new(addr, &provider);
        let bn = provider.get_block_number().await.expect("bn");
        let block = alloy::eips::BlockId::number(bn);

        let r0 = curve.balances(0i128).block(block).call().await.expect("b0");
        let r1 = curve.balances(1i128).block(block).call().await.expect("b1");
        let r2 = curve.balances(2i128).block(block).call().await.expect("b2");
        let r3 = curve.balances(3i128).block(block).call().await.expect("b3");
        let raw_a = curve.A().block(block).call().await.expect("A");
        let pool_fee = curve.fee().block(block).call().await.expect("fee");

        let rate18 = U256::from(1_000_000_000_000_000_000u128);
        let rate6 = U256::from(1_000_000_000_000_000_000_000_000_000_000u128);

        let dx = U256::from(100_000_000_000_000_000u128);
        let on_chain = curve
            .get_dy(0i128, 1i128, dx)
            .block(block)
            .call()
            .await
            .expect("dy");
        let ours = get_amount_out(
            &[r0, r1, r2, r3],
            &[rate18, rate6, rate6, rate18],
            raw_a,
            pool_fee,
            0,
            1,
            dx,
        )
        .expect("ours");
        println!("  DAI→USDC: on_chain={on_chain}, ours={ours}");
        assert_eq!(ours, on_chain, "mismatch");
        println!("StableSwapV0 (sUSD) passed!");
    }
}
