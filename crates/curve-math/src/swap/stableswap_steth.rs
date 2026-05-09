//! Pool-level swap math for the Lido stETH/ETH pool.
//!
//! Identical to [`crate::swap::stableswap_v2`] except `get_d` comes from
//! [`crate::core::stableswap_steth`] (the `+1` quirk). `get_y`, fee handling,
//! `-1` offset and denormalization are unchanged from V2.

use alloy_primitives::U256;

use crate::core::stableswap_steth::{get_d, get_y, A_PRECISION, FEE_DENOMINATOR, PRECISION};

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
    let dy_after_fee_internal = desired_output * rates[j] / precision;
    let complement = fee_denom - fee;
    let dy_internal = (dy_after_fee_internal * fee_denom + complement - U256::from(1)) / complement;
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
    // D_P matches the on-chain quirk loop for the spot price derivative as
    // well, since the implicit-differentiation expression depends on D.
    let mut d_p = d;
    for x_k in &xp {
        d_p = d_p
            .checked_mul(d)?
            .checked_div(x_k.checked_mul(n)?.checked_add(U256::from(1))?)?;
    }
    let num_xp = ann_eff.checked_mul(xp[i])?.checked_add(d_p)?;
    let den_xp = ann_eff.checked_mul(xp[j])?.checked_add(d_p)?;
    if den_xp.is_zero() {
        return None;
    }
    let numerator = num_xp
        .checked_mul(balances[j])?
        .checked_mul(fee_denom - fee)?;
    let denominator = den_xp.checked_mul(balances[i])?.checked_mul(fee_denom)?;
    Some((numerator, denominator))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wei-exact match against on-chain `get_dy(0, 1, 1) = 0` at block 25058715.
    #[test]
    fn steth_dx_one_returns_zero() {
        let balances = [
            U256::from_str_radix("17705718241149099908355", 10).unwrap(),
            U256::from_str_radix("24165038077063260153277", 10).unwrap(),
        ];
        let rate18 = U256::from(1_000_000_000_000_000_000u128);
        let rates = [rate18, rate18];
        let amp = U256::from(90_000u64);
        let fee = U256::from(1_000_000u64);
        // dx=1: on-chain returns 0 because dy = xp[1] - y - 1 = 0 with the +1 quirk.
        let result = get_amount_out(&balances, &rates, amp, fee, 0, 1, U256::from(1u64));
        assert_eq!(result, None);
    }

    /// Wei-exact match against on-chain `get_dy(0, 1, 1e18) = 1000259270666916332`.
    #[test]
    fn steth_dx_one_eth() {
        let balances = [
            U256::from_str_radix("17705718241149099908355", 10).unwrap(),
            U256::from_str_radix("24165038077063260153277", 10).unwrap(),
        ];
        let rate18 = U256::from(1_000_000_000_000_000_000u128);
        let rates = [rate18, rate18];
        let amp = U256::from(90_000u64);
        let fee = U256::from(1_000_000u64);
        let result = get_amount_out(
            &balances,
            &rates,
            amp,
            fee,
            0,
            1,
            U256::from(1_000_000_000_000_000_000u128),
        )
        .unwrap();
        assert_eq!(result, U256::from(1_000_259_270_666_916_332u128));
    }
}
