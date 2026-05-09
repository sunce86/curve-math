//! StableSwapSTETH — Lido stETH/ETH (`0xDC24316b9AE028F1497c275EB9192a3Ea0f67022`).
//!
//! Custom Vyper deployment, not factory-derived. Math matches `pool-templates/base`
//! (StableSwapV2) except for one quirk in `get_D`:
//!
//! ```text
//! D_P = D_P * D / (_x * N_COINS + 1)  # +1 is to prevent /0
//! ```
//!
//! That single `+1` shifts D by 1 wei, which propagates through the Newton solver
//! and produces wei-level mismatches at small `dx` for V2-modeled callers.
//!
//! Newton `get_y` is identical to V2 and is re-exported as is.
//!
//! Vyper: <https://github.com/curvefi/curve-contract/blob/master/contracts/pools/steth/StableSwapSTETH.vy>

use alloy_primitives::U256;

pub use crate::core::stableswap_v2::{get_y, A_PRECISION, FEE_DENOMINATOR, PRECISION};

const MAX_ITERATIONS: usize = 255;

pub fn get_d(xp: &[U256], amp: U256) -> Option<U256> {
    let n = U256::from(xp.len());

    let sum: U256 = xp
        .iter()
        .try_fold(U256::ZERO, |acc, b| acc.checked_add(*b))?;
    if sum.is_zero() {
        return Some(U256::ZERO);
    }

    let ann = amp.checked_mul(n)?;
    let mut d = sum;

    for _ in 0..MAX_ITERATIONS {
        let mut d_p = d;
        for balance in xp {
            d_p = d_p
                .checked_mul(d)?
                .checked_div(balance.checked_mul(n)?.checked_add(U256::from(1))?)?;
        }

        let d_prev = d;

        let numerator = ann
            .checked_mul(sum)?
            .checked_div(A_PRECISION)?
            .checked_add(d_p.checked_mul(n)?)?
            .checked_mul(d)?;

        let denominator = ann
            .checked_sub(A_PRECISION)?
            .checked_mul(d)?
            .checked_div(A_PRECISION)?
            .checked_add(n.checked_add(U256::from(1))?.checked_mul(d_p)?)?;

        if denominator.is_zero() {
            return None;
        }

        d = numerator.checked_div(denominator)?;

        let diff = if d > d_prev { d - d_prev } else { d_prev - d };
        if diff <= U256::from(1) {
            return Some(d);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lido stETH/ETH at block 25058715: confirms wei-exact match with on-chain
    /// `get_dy(0, 1, 1) = 0`. With the canonical V2 `get_d` (no `+1`), our
    /// Newton solver converges to xp[1] - 2 instead of xp[1] - 1, producing
    /// dy = 1 (off by 1 wei) for tiny dx.
    #[test]
    fn steth_block_25058715() {
        let xp = [
            U256::from_str_radix("17705718241149099908355", 10).unwrap(),
            U256::from_str_radix("24165038077063260153277", 10).unwrap(),
        ];
        let amp = U256::from(90_000u64); // _A() at block 25058715 (post-ramp)
        let d = get_d(&xp, amp).expect("converges");
        let y = get_y(0, 1, xp[0] + U256::from(1u64), &xp, d, amp).expect("converges");
        assert_eq!(xp[1] - y - U256::from(1u64), U256::ZERO);
    }
}
