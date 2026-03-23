//! Pool-level get_amount_out for StableSwapALend (Aave, sAAVE, IB, aETH).
//!
//! NO -1. Denorm FIRST, then dynamic fee with avg xp.
//! Uses PRECISION_MUL (not stored_rates/PRECISION).

use alloy_primitives::U256;

use crate::core::stableswap_alend::{dynamic_fee, get_d, get_y, A_PRECISION, FEE_DENOMINATOR};

pub fn get_amount_out(
    balances: &[U256],
    precision_mul: &[U256],
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
    // Vyper: xp = _balances(); for k: xp[k] *= precisions[k]
    let xp: Vec<U256> = balances
        .iter()
        .zip(precision_mul.iter())
        .map(|(b, p)| *b * *p)
        .collect();
    let d = get_d(&xp, amp)?;
    // Vyper: x = xp[i] + _dx * precisions[i]
    let x_new = xp[i] + dx * precision_mul[i];
    let y_new = get_y(i, j, x_new, &xp, d, amp)?;
    if xp[j] <= y_new {
        return None;
    }
    // NO -1. Denorm FIRST: dy = (xp[j] - y) / precisions[j]
    let dy = (xp[j] - y_new) / precision_mul[j];
    // Dynamic fee with avg xp
    let fee_rate = dynamic_fee(
        (xp[i] + x_new) / U256::from(2),
        (xp[j] + y_new) / U256::from(2),
        fee,
        offpeg_fee_multiplier,
    );
    let fee_amount = fee_rate * dy / U256::from(FEE_DENOMINATOR);
    let result = dy - fee_amount;
    if result.is_zero() {
        return None;
    }
    Some(result)
}

pub fn get_amount_in(
    balances: &[U256],
    precision_mul: &[U256],
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
    let fee_denom = U256::from(FEE_DENOMINATOR);
    let xp: Vec<U256> = balances
        .iter()
        .zip(precision_mul.iter())
        .map(|(b, p)| *b * *p)
        .collect();
    let d = get_d(&xp, amp)?;

    // First pass: use base fee as estimate (round up)
    let fee_complement = fee_denom - fee;
    let dy = (desired_output * fee_denom + fee_complement - U256::from(1)) / fee_complement;
    let dy_internal = dy * precision_mul[j];
    if xp[j] <= dy_internal {
        return None;
    }
    let y_new = xp[j] - dy_internal;
    let x_new = get_y(j, i, y_new, &xp, d, amp)?;

    // Second pass: recompute with actual dynamic fee
    let actual_fee = dynamic_fee(
        (xp[i] + x_new) / U256::from(2),
        (xp[j] + y_new) / U256::from(2),
        fee,
        offpeg_fee_multiplier,
    );
    let actual_complement = fee_denom - actual_fee;
    let dy = (desired_output * fee_denom + actual_complement - U256::from(1)) / actual_complement;
    let dy_internal = dy * precision_mul[j];
    if xp[j] <= dy_internal {
        return None;
    }
    let y_new = xp[j] - dy_internal;
    let x_new = get_y(j, i, y_new, &xp, d, amp)?;
    if x_new <= xp[i] {
        return None;
    }
    let dx = (x_new - xp[i]) / precision_mul[i] + U256::from(1);
    Some(dx)
}

/// Spot price dy/dx including fee, returned as (numerator, denominator).
/// Analytical: from implicit differentiation of StableSwap invariant.
/// Uses dynamic fee at current pool state (zero trade size).
pub fn spot_price(
    balances: &[U256],
    precision_mul: &[U256],
    amp: U256,
    fee: U256,
    offpeg_fee_multiplier: U256,
    i: usize,
    j: usize,
) -> Option<(U256, U256)> {
    let fee_denom = U256::from(FEE_DENOMINATOR);
    let n = U256::from(balances.len());
    let ann_eff = amp.checked_mul(n)? / U256::from(A_PRECISION);
    let xp: Vec<U256> = balances
        .iter()
        .zip(precision_mul.iter())
        .map(|(b, p)| *b * *p)
        .collect();
    let d = get_d(&xp, amp)?;
    // D_P = D^(N+1) / (N^N * prod(xp)), computed iteratively
    let mut d_p = d;
    for x_k in &xp {
        d_p = d_p.checked_mul(d)?.checked_div(x_k.checked_mul(n)?)?;
    }
    // Implicit differentiation of StableSwap invariant at constant D:
    // dy/dx = (A_n + D_P/xp[i]) / (A_n + D_P/xp[j])
    // As integer fraction: (A_n*xp[i] + D_P) * bal[j] / ((A_n*xp[j] + D_P) * bal[i])
    let num_xp = ann_eff.checked_mul(xp[i])?.checked_add(d_p)?;
    let den_xp = ann_eff.checked_mul(xp[j])?.checked_add(d_p)?;
    if den_xp.is_zero() {
        return None;
    }
    let effective_fee = dynamic_fee(xp[i], xp[j], fee, offpeg_fee_multiplier);
    let numerator = num_xp
        .checked_mul(balances[j])?
        .checked_mul(fee_denom - effective_fee)?;
    let denominator = den_xp.checked_mul(balances[i])?.checked_mul(fee_denom)?;
    Some((numerator, denominator))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::stableswap_alend::A_PRECISION;

    #[test]
    fn roundtrip() {
        // All 18-dec tokens (precision_mul = 1)
        let balances = [
            U256::from(10_000_000_000_000_000_000_000_000u128),
            U256::from(10_000_000_000_000_000_000_000_000u128),
            U256::from(10_000_000_000_000_000_000_000_000u128),
        ];
        let prec_mul = [U256::from(1u64), U256::from(1u64), U256::from(1u64)];
        let amp = U256::from(100u64 * A_PRECISION as u64);
        let fee = U256::from(4_000_000u64);
        let offpeg = U256::from(20_000_000_000u64);
        let dx = U256::from(1_000_000_000_000_000_000_000u128);
        let dy = get_amount_out(&balances, &prec_mul, amp, fee, offpeg, 0, 1, dx).expect("out");
        let dx_recovered =
            get_amount_in(&balances, &prec_mul, amp, fee, offpeg, 0, 1, dy).expect("in");
        assert!(dx_recovered >= dx);
        assert!(dx_recovered <= dx + U256::from(2));
        let dy_check = get_amount_out(&balances, &prec_mul, amp, fee, offpeg, 0, 1, dx_recovered)
            .expect("check");
        assert!(dy_check >= dy);
    }

    #[test]
    fn spot_price_balanced_near_one() {
        let balances = [
            U256::from(10_000_000_000_000_000_000_000_000u128),
            U256::from(10_000_000_000_000_000_000_000_000u128),
            U256::from(10_000_000_000_000_000_000_000_000u128),
        ];
        let prec_mul = [U256::from(1u64), U256::from(1u64), U256::from(1u64)];
        let amp = U256::from(100u64 * A_PRECISION as u64);
        let fee = U256::from(4_000_000u64);
        let offpeg = U256::from(20_000_000_000u64);
        let (num, den) = spot_price(&balances, &prec_mul, amp, fee, offpeg, 0, 1).expect("price");
        let diff = if num > den { num - den } else { den - num };
        assert!(diff * U256::from(1000) < den, "spot price not near 1");
    }

    #[test]
    fn spot_price_symmetry() {
        let balances = [
            U256::from(10_000_000_000_000_000_000_000_000u128),
            U256::from(10_000_000_000_000_000_000_000_000u128),
            U256::from(10_000_000_000_000_000_000_000_000u128),
        ];
        let prec_mul = [U256::from(1u64), U256::from(1u64), U256::from(1u64)];
        let amp = U256::from(100u64 * A_PRECISION as u64);
        let fee = U256::from(4_000_000u64);
        let offpeg = U256::from(20_000_000_000u64);
        let (num_ij, den_ij) =
            spot_price(&balances, &prec_mul, amp, fee, offpeg, 0, 1).expect("price_ij");
        let (num_ji, den_ji) =
            spot_price(&balances, &prec_mul, amp, fee, offpeg, 1, 0).expect("price_ji");
        // price(i,j) * price(j,i) ≈ (1-f)^2
        // Compare via cross-mul to avoid overflow: num_ij * den_ji ≈ num_ji * den_ij
        // (for balanced pool, both prices ≈ (1-f), so their ratio should be ≈ 1)
        let lhs = num_ij * den_ji;
        let rhs = num_ji * den_ij;
        let diff = if lhs > rhs { lhs - rhs } else { rhs - lhs };
        assert!(diff * U256::from(1000) < rhs, "symmetry violated");
    }

    #[test]
    fn spot_price_consistent_with_swap() {
        let balances = [
            U256::from(10_000_000_000_000_000_000_000_000u128),
            U256::from(10_000_000_000_000_000_000_000_000u128),
            U256::from(10_000_000_000_000_000_000_000_000u128),
        ];
        let prec_mul = [U256::from(1u64), U256::from(1u64), U256::from(1u64)];
        let amp = U256::from(100u64 * A_PRECISION as u64);
        let fee = U256::from(4_000_000u64);
        let offpeg = U256::from(20_000_000_000u64);
        let dx = U256::from(1_000_000_000_000_000u128);
        let dy = get_amount_out(&balances, &prec_mul, amp, fee, offpeg, 0, 1, dx).expect("out");
        let (num, den) = spot_price(&balances, &prec_mul, amp, fee, offpeg, 0, 1).expect("price");
        let lhs = dy * den;
        let rhs = dx * num;
        let diff = if lhs > rhs { lhs - rhs } else { rhs - lhs };
        assert!(
            diff * U256::from(100) < rhs,
            "spot price inconsistent with swap"
        );
    }
}
