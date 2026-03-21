//! Pool-level get_amount_out for StableSwapNG (plain + meta NG pools).
//!
//! -1 offset. Fee before denormalize. Dynamic fee with avg xp.

use alloy_primitives::U256;

use crate::core::stableswap_ng::{
    dynamic_fee, get_d, get_y, A_PRECISION, FEE_DENOMINATOR, PRECISION,
};

pub fn get_amount_out(
    balances: &[U256],
    rates: &[U256],
    amp: U256,
    fee: U256,
    offpeg_fee_multiplier: U256,
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

    // Vyper: dy = xp[j] - y - 1
    let dy = xp[j] - y_new - U256::from(1);

    // Vyper: fee = _dynamic_fee((xp[i]+x)/2, (xp[j]+y)/2, ...) * dy / FEE_DENOM
    let fee_rate = dynamic_fee(
        (xp[i] + x_new) / U256::from(2u64),
        (xp[j] + y_new) / U256::from(2u64),
        fee,
        offpeg_fee_multiplier,
    );
    let fee_amount = fee_rate * dy / U256::from(FEE_DENOMINATOR);

    // Vyper: return (dy - fee) * PRECISION / rates[j]
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
    offpeg_fee_multiplier: U256,
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

    // Reverse denorm
    let dy_after_fee_internal = desired_output * rates[j] / precision;

    // First pass: use base fee as estimate (round up)
    let fee_complement = fee_denom - fee;
    let dy_internal =
        (dy_after_fee_internal * fee_denom + fee_complement - U256::from(1)) / fee_complement;
    if xp[j] <= dy_internal + U256::from(1) {
        return None;
    }
    let y_new = xp[j] - dy_internal - U256::from(1);
    let x_new = get_y(j, i, y_new, &xp, d, amp)?;

    // Second pass: recompute with actual dynamic fee
    let actual_fee = dynamic_fee(
        (xp[i] + x_new) / U256::from(2u64),
        (xp[j] + y_new) / U256::from(2u64),
        fee,
        offpeg_fee_multiplier,
    );
    let actual_complement = fee_denom - actual_fee;
    let dy_internal =
        (dy_after_fee_internal * fee_denom + actual_complement - U256::from(1)) / actual_complement;
    if xp[j] <= dy_internal + U256::from(1) {
        return None;
    }
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
/// Uses dynamic fee at current pool state (zero trade size).
pub fn spot_price(
    balances: &[U256],
    rates: &[U256],
    amp: U256,
    fee: U256,
    offpeg_fee_multiplier: U256,
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
    // Dynamic fee at current pool state
    let effective_fee = dynamic_fee(xp[i], xp[j], fee, offpeg_fee_multiplier);
    // Include fee: multiply numerator by (FEE_DENOM - fee)
    let numerator = num_xp * rates[i] * (fee_denom - effective_fee);
    let denominator = den_xp * rates[j] * fee_denom;
    Some((numerator, denominator))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::stableswap_ng::A_PRECISION;

    const RATE_18: u128 = 1_000_000_000_000_000_000;

    #[test]
    fn basic_swap() {
        let b = U256::from(1_000_000_000_000_000_000_000_000u128);
        let out = get_amount_out(
            &[b, b],
            &[U256::from(RATE_18), U256::from(RATE_18)],
            U256::from(400u64 * A_PRECISION as u64),
            U256::from(4_000_000u64),
            U256::from(20_000_000_000u64),
            0,
            1,
            U256::from(1_000_000_000_000_000_000_000u128),
        )
        .expect("swap");
        assert!(out > U256::ZERO);
    }

    #[test]
    fn roundtrip() {
        let b = U256::from(1_000_000_000_000_000_000_000_000u128);
        let balances = [b, b];
        let rates = [U256::from(RATE_18), U256::from(RATE_18)];
        let amp = U256::from(400u64 * A_PRECISION as u64);
        let fee = U256::from(4_000_000u64);
        let offpeg = U256::from(20_000_000_000u64);
        let dx = U256::from(1_000_000_000_000_000_000_000u128);
        let dy = get_amount_out(&balances, &rates, amp, fee, offpeg, 0, 1, dx).expect("out");
        let dx_recovered =
            get_amount_in(&balances, &rates, amp, fee, offpeg, 0, 1, dy).expect("in");
        assert!(dx_recovered >= dx);
        assert!(dx_recovered <= dx + U256::from(2));
        let dy_check =
            get_amount_out(&balances, &rates, amp, fee, offpeg, 0, 1, dx_recovered).expect("check");
        assert!(dy_check >= dy);
    }

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
    #[ignore = "requires RPC_URL env var pointing to Ethereum mainnet"]
    async fn verify_usde_dai() {
        use alloy::providers::{Provider, ProviderBuilder};
        use std::str::FromStr;

        let rpc_url = std::env::var("RPC_URL").expect("RPC_URL must be set");
        let provider =
            ProviderBuilder::new().connect_http(rpc_url.parse().expect("invalid RPC_URL"));

        let pool_address =
            alloy_primitives::Address::from_str("0xF36a4BA50C603204c3FC6d2dA8b78A7b69CBC67d")
                .expect("valid");
        let curve = ICurvePoolNG::new(pool_address, &provider);

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
        let offpeg = curve
            .offpeg_fee_multiplier()
            .block(block)
            .call()
            .await
            .expect("offpeg");

        let balances = [r0, r1];
        let rates = [U256::from(RATE_18), U256::from(RATE_18)];
        let amp = raw_a * U256::from(A_PRECISION as u64);

        let dx = U256::from(1_000_000_000_000_000_000u128);
        let on_chain = curve
            .get_dy(0i128, 1i128, dx)
            .block(block)
            .call()
            .await
            .expect("get_dy");
        let ours =
            get_amount_out(&balances, &rates, amp, pool_fee, offpeg, 0, 1, dx).expect("ours");
        println!("  USDe→DAI: on_chain={on_chain}, ours={ours}");
        assert_eq!(ours, on_chain, "USDe→DAI mismatch");

        println!("StableSwapNG (USDe/DAI) verification passed!");
    }
}
