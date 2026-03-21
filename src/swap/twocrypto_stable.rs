//! Pool-level get_amount_out for TwoCryptoNG pools using StableSwap MATH (v0.1.0).
//!
//! These pools are deployed from the TwoCryptoNG factory but use StableSwap invariant.
//! Normalization and fee are CryptoSwap-style (price_scale, crypto_fee).
//! Core math is StableSwap (gamma is ignored).

use alloy_primitives::U256;

use crate::core::twocrypto_stable::get_y;

pub const WAD: u128 = 1_000_000_000_000_000_000;
pub const FEE_DENOMINATOR: u64 = 10_000_000_000;

fn crypto_fee(xp: &[U256], mid_fee: U256, out_fee: U256, fee_gamma: U256) -> Option<U256> {
    let wad = U256::from(WAD);
    let s: U256 = xp.iter().try_fold(U256::ZERO, |acc, v| acc.checked_add(*v))?;
    if s.is_zero() { return None; }
    let n = U256::from(xp.len());
    // Vyper V1 _fee: K = WAD * N^N * xp[0] / S * xp[1] / S
    let mut nn = U256::from(1u64);
    for _ in 0..xp.len() { nn = nn * n; }
    let mut k = wad * nn;
    for x_i in xp { k = k * (*x_i) / s; }
    // NG fee formula: f = fee_gamma * k / (fee_gamma * k / WAD + WAD - k)
    let f = if fee_gamma > U256::ZERO {
        fee_gamma * k / (fee_gamma * k / wad + wad - k)
    } else { k };
    Some((mid_fee * f + out_fee * (wad - f)) / wad)
}

pub fn get_amount_out(
    balances: &[U256; 2],
    precisions: &[U256; 2],
    price_scale: U256,
    d: U256,
    ann: U256,
    mid_fee: U256,
    out_fee: U256,
    fee_gamma: U256,
    i: usize,
    j: usize,
    dx: U256,
) -> Option<U256> {
    if dx.is_zero() {
        return None;
    }

    let wad = U256::from(WAD);
    let price_scale_local = price_scale * precisions[1];

    // CryptoSwap normalization — compute xp BEFORE and AFTER dx
    let xp_orig: [U256; 2] = [balances[0] * precisions[0], balances[1] * price_scale_local / wad];
    let mut bal = *balances;
    bal[i] += dx;
    let xp: [U256; 2] = [bal[0] * precisions[0], bal[1] * price_scale_local / wad];

    // StableSwap core — use stored D (passed from on-chain), get_y with new xp[i]
    let y_new = get_y(i, j, xp[i], &xp_orig, d, ann)?;

    if xp[j] <= y_new {
        return None;
    }

    let dy = xp[j] - y_new - U256::from(1);
    let xp_after: [U256; 2] = if j == 0 { [y_new, xp[1]] } else { [xp[0], y_new] };

    // CryptoSwap denormalization
    let dy_native = if j > 0 {
        dy * wad / price_scale / precisions[j]
    } else {
        dy / precisions[0]
    };

    // CryptoSwap fee
    let fee = crypto_fee(&xp_after, mid_fee, out_fee, fee_gamma)?;
    let fee_amount = fee * dy_native / U256::from(FEE_DENOMINATOR);
    let result = dy_native - fee_amount;

    if result.is_zero() {
        return None;
    }

    Some(result)
}

/// Spot price dy/dx including fee, returned as (numerator, denominator).
/// Numerical: compute get_amount_out with a small dx for marginal price.
pub fn spot_price(
    balances: &[U256; 2],
    precisions: &[U256; 2],
    price_scale: U256,
    d: U256,
    ann: U256,
    mid_fee: U256,
    out_fee: U256,
    fee_gamma: U256,
    i: usize,
    j: usize,
) -> Option<(U256, U256)> {
    let dx = U256::from(1_000_000_000_000_000u64);
    let dy = get_amount_out(
        balances, precisions, price_scale, d, ann,
        mid_fee, out_fee, fee_gamma, i, j, dx,
    )?;
    Some((dy, dx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_swap() {
        let wad = U256::from(1_000_000_000_000_000_000u128);
        // Realistic params from crvUSD/WETH pool using MATH v0.1.0
        let balances = [
            U256::from_str_radix("16913758444575087263963692", 10).unwrap(),
            U256::from_str_radix("13702674829211830926174", 10).unwrap(),
        ];
        let precisions = [U256::from(1u64), U256::from(1u64)];
        let price_scale = U256::from_str_radix("2797050192554649390777", 10).unwrap();
        let ann = U256::from(25000u64);
        let mid_fee = U256::from(60000000u64);
        let out_fee = U256::from(220000000u64);
        let fee_gamma = U256::from_str_radix("1395000000000000", 10).unwrap();
        // Compute D from xp (simulating stored D)
        let price_scale_local = price_scale * precisions[1];
        let xp_orig: [U256; 2] = [balances[0] * precisions[0], balances[1] * price_scale_local / wad];
        let d = crate::core::twocrypto_stable::get_d(&xp_orig, ann).expect("D");
        let dx = wad; // 1 crvUSD
        let dy = get_amount_out(
            &balances, &precisions, price_scale, d, ann,
            mid_fee, out_fee, fee_gamma, 0, 1, dx,
        ).expect("swap");
        assert!(dy > U256::ZERO);
    }
}
