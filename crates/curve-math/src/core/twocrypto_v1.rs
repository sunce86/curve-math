//! TwoCryptoV1 — Legacy 2-coin CryptoSwap (CurveCryptoSwap2ETH).
//!
//! Vyper: https://github.com/curvefi/curve-crypto-contract/blob/master/contracts/two/CurveCryptoSwap2ETH.vy
//!
//! NOTE: The repo also contains CurveCryptoSwap2.vy (non-ETH variant) which has a different
//! `mul2` formula: `unsafe_div(10**18 + 2*10**18*K0, _g1k0)` (divides entire sum).
//! The ETH variant uses: `10**18 + (2*10**18)*K0 / _g1k0` (divides only second term).
//! Deployed CRV/ETH pool matches the ETH variant.

use alloy_primitives::U256;

pub const WAD: U256 = U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);
pub const FEE_DENOMINATOR: U256 = U256::from_limbs([10_000_000_000, 0, 0, 0]);
pub const A_MULTIPLIER: U256 = U256::from_limbs([10_000, 0, 0, 0]);
const MAX_ITERATIONS: usize = 255;

pub fn newton_y_2(ann: U256, gamma: U256, x: [U256; 2], d: U256, j: usize) -> Option<U256> {
    let n = U256::from(2u64);
    let x_j = x[1 - j];
    let mut y = d * d / (x_j * U256::from(4u64));
    let k0_i = WAD * n * x_j / d;
    let convergence_limit = {
        let a = x_j / U256::from(10u128.pow(14));
        let b = d / U256::from(10u128.pow(14));
        a.max(b).max(U256::from(100u64))
    };
    let __g1k0 = gamma + WAD;
    for _ in 0..MAX_ITERATIONS {
        let y_prev = y;
        let k0 = k0_i * y * n / d;
        let s = x_j + y;
        let _g1k0 = if __g1k0 > k0 {
            __g1k0 - k0 + U256::from(1)
        } else {
            k0 - __g1k0 + U256::from(1)
        };
        let mul1 = WAD * d / gamma * _g1k0 / gamma * _g1k0 * A_MULTIPLIER / ann;
        let mul2 = WAD + U256::from(2u64) * WAD * k0 / _g1k0;
        let yfprime = WAD * y + s * mul2 + mul1;
        let _dyfprime = d * mul2;
        if yfprime < _dyfprime {
            y = y_prev / U256::from(2);
            continue;
        }
        let yfprime = yfprime - _dyfprime;
        let fprime = yfprime / y;
        let y_minus = mul1 / fprime;
        let y_plus = (yfprime + WAD * d) / fprime + y_minus * WAD / k0;
        let y_minus = y_minus + WAD * s / fprime;
        if y_plus < y_minus {
            y = y_prev / U256::from(2);
        } else {
            y = y_plus - y_minus;
        }
        let diff = if y > y_prev { y - y_prev } else { y_prev - y };
        if diff < convergence_limit.max(y / U256::from(10u128.pow(14))) {
            let frac = y * WAD / d;
            if frac < U256::from(10u128.pow(16)) || frac > U256::from(10u128.pow(20)) {
                return None;
            }
            return Some(y);
        }
    }
    None
}

pub fn crypto_fee(xp: &[U256], mid_fee: U256, out_fee: U256, fee_gamma: U256) -> Option<U256> {
    let s: U256 = xp
        .iter()
        .try_fold(U256::ZERO, |acc, v| acc.checked_add(*v))?;
    if s.is_zero() {
        return None;
    }
    // Vyper V1 _fee computes K inline: (WAD * N^N) * xp[0] / S * xp[1] / S
    // NOT per-element k *= N * x_i / S (which differs in integer truncation)
    let n = U256::from(xp.len());
    let mut nn = U256::from(1u64);
    for _ in 0..xp.len() {
        nn *= n;
    }
    let mut k = WAD * nn;
    for x_i in xp {
        k = k * (*x_i) / s;
    }
    let f = if fee_gamma > U256::ZERO {
        fee_gamma * WAD / (fee_gamma + WAD - k)
    } else {
        k
    };
    Some((mid_fee * f + out_fee * (WAD - f)) / WAD)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Realistic CRV/ETH pool parameters (both 18-dec, normalized to internal space)
    fn realistic_params() -> (U256, U256, [U256; 2], U256) {
        let wad = WAD;
        // A=540000, gamma=2.8e13 (typical twocrypto V1)
        let ann = U256::from(540_000u64) * A_MULTIPLIER;
        let gamma = U256::from(28_000_000_000_000u64);
        // ~5000 ETH and ~20M CRV normalized
        let x0 = U256::from(5000u64) * wad;
        let x1 = U256::from(5000u64) * wad; // price_scale-normalized
        let d = U256::from(10000u64) * wad;
        (ann, gamma, [x0, x1], d)
    }

    #[test]
    fn newton_y_2_convergence() {
        let (ann, gamma, x, d) = realistic_params();
        let y = newton_y_2(ann, gamma, x, d, 1).expect("converge");
        assert!(y > U256::ZERO);
        assert!(y < d);
    }

    #[test]
    fn newton_y_2_with_swap() {
        let wad = WAD;
        let (ann, gamma, x, d) = realistic_params();
        let dx = U256::from(10u64) * wad;
        let y_before = newton_y_2(ann, gamma, x, d, 1).expect("before");
        let y_after = newton_y_2(ann, gamma, [x[0] + dx, x[1]], d, 1).expect("after");
        assert!(y_after < y_before);
    }

    #[test]
    fn crypto_fee_balanced() {
        let wad = WAD;
        let mid_fee = U256::from(3_000_000u64);
        let out_fee = U256::from(30_000_000u64);
        let fee_gamma = U256::from(230_000_000_000_000u64);
        let xp = [U256::from(100_000u64) * wad, U256::from(100_000u64) * wad];
        let fee = crypto_fee(&xp, mid_fee, out_fee, fee_gamma).expect("fee");
        // Balanced pool: k ≈ 1, f ≈ 1, fee ≈ mid_fee
        assert!(fee >= mid_fee);
        assert!(fee < out_fee);
    }

    #[test]
    fn crypto_fee_imbalanced() {
        let wad = WAD;
        let mid_fee = U256::from(3_000_000u64);
        let out_fee = U256::from(30_000_000u64);
        let fee_gamma = U256::from(230_000_000_000_000u64);
        let balanced = [U256::from(100_000u64) * wad, U256::from(100_000u64) * wad];
        let imbalanced = [U256::from(200_000u64) * wad, U256::from(50_000u64) * wad];
        let fee_b = crypto_fee(&balanced, mid_fee, out_fee, fee_gamma).expect("balanced");
        let fee_i = crypto_fee(&imbalanced, mid_fee, out_fee, fee_gamma).expect("imbalanced");
        // Imbalanced should have higher fee
        assert!(fee_i > fee_b);
    }
}
