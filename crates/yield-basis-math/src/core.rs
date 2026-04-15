//! Core stateless math for Yield Basis LEVAMM.
//!
//! Ported from `AMM.vy`: <https://github.com/yield-basis/yb-core/blob/main/contracts/AMM.vy>

use alloy_primitives::U256;

use crate::constants::WAD;

/// Integer square root (floor). Babylonian method.
/// Matches Vyper's `isqrt()`.
pub fn sqrt(x: U256) -> U256 {
    if x.is_zero() {
        return U256::ZERO;
    }
    let mut z = (x + U256::from(1u64)) >> 1;
    let mut y = x;
    while z < y {
        y = z;
        z = (x / z + z) >> 1;
    }
    y
}

/// Ceiling division: `ceil(a / b)`.
/// Matches Vyper's `math._ceil_div(a, b)`.
pub fn ceil_div(a: U256, b: U256) -> U256 {
    (a + b - U256::from(1u64)) / b
}

/// Compute `x0` — the virtual reserve invariant of the LEVAMM.
///
/// `AMM.vy` L142-158: `get_x0(p_oracle, collateral, debt, safe_limits)`
///
/// Returns `None` if the discriminant is negative (debt too large) or
/// `safe_limits` check fails.
///
/// # Arguments
/// * `p_oracle` — oracle price in WAD
/// * `collateral` — collateral amount in native decimals
/// * `debt` — current debt in stablecoin (18 decimals)
/// * `collateral_precision` — `10^(18 - collateral_decimals)`
/// * `lev_ratio` — `L^2 * 1e18 / (2L - 1)^2`, precomputed
/// * `safe_limits` — enforce min/max debt bounds
/// * `min_safe_debt` — `1e54 / (4 * L^2)`
/// * `max_safe_debt` — `(2L-1)^2 * 1e18 / (4*L^2) - 1e54 / (8*L^2)`
#[allow(clippy::too_many_arguments)]
pub fn get_x0(
    p_oracle: U256,
    collateral: U256,
    debt: U256,
    collateral_precision: U256,
    lev_ratio: U256,
    safe_limits: bool,
    min_safe_debt: U256,
    max_safe_debt: U256,
) -> Option<U256> {
    // coll_value = p_oracle * collateral * COLLATERAL_PRECISION / 1e18
    let coll_value = p_oracle * collateral * collateral_precision / WAD;

    if safe_limits {
        // debt >= coll_value * MIN_SAFE_DEBT / 1e18
        if debt < coll_value * min_safe_debt / WAD {
            return None;
        }
        // debt <= coll_value * MAX_SAFE_DEBT / 1e18
        if debt > coll_value * max_safe_debt / WAD {
            return None;
        }
    }

    // D = coll_value^2 - 4 * coll_value * LEV_RATIO / 1e18 * debt
    // Safe: WAD = 1e18 ≠ 0, lev_ratio > 0 (checked by YieldBasisPool::new)
    let four_lr_debt = U256::from(4u64) * coll_value * lev_ratio / WAD * debt;
    let cv_sq = coll_value * coll_value;
    if cv_sq < four_lr_debt {
        return None; // negative discriminant
    }
    let d = cv_sq - four_lr_debt;

    // x0 = (coll_value + sqrt(D)) * 1e18 / (2 * LEV_RATIO)
    // Safe: lev_ratio > 0 (constructor invariant)
    Some((coll_value + sqrt(d)) * WAD / (U256::from(2u64) * lev_ratio))
}

/// Compute rate multiplier from stored values and current timestamp.
///
/// `AMM.vy` L163-169: `_rate_mul()`
///
/// `rate_mul_stored * (1e18 + rate * (now - rate_time)) / 1e18`
pub fn compute_rate_mul(rate_mul_stored: U256, rate: U256, rate_time: U256, now: U256) -> U256 {
    if now <= rate_time {
        return rate_mul_stored;
    }
    // Safe: WAD = 1e18 ≠ 0
    rate_mul_stored * (WAD + rate * (now - rate_time)) / WAD
}

/// Compute current debt from stored debt and rate multipliers.
///
/// `AMM.vy` L202-203: `self.debt * self._rate_mul() // self.rate_mul`
///
/// Stored debt is in raw units (not WAD-normalized). Division is by
/// `stored_rate_mul`, not WAD.
pub fn compute_debt(stored_debt: U256, current_rate_mul: U256, stored_rate_mul: U256) -> U256 {
    if stored_rate_mul.is_zero() {
        return stored_debt;
    }
    stored_debt * current_rate_mul / stored_rate_mul
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqrt_basic() {
        assert_eq!(sqrt(U256::ZERO), U256::ZERO);
        assert_eq!(sqrt(U256::from(1u64)), U256::from(1u64));
        assert_eq!(sqrt(U256::from(4u64)), U256::from(2u64));
        assert_eq!(sqrt(U256::from(10u64)), U256::from(3u64));
    }

    #[test]
    fn sqrt_large() {
        let x = U256::from(10u64).pow(U256::from(36));
        assert_eq!(sqrt(x), U256::from(10u64).pow(U256::from(18)));
    }

    #[test]
    fn ceil_div_basic() {
        assert_eq!(
            ceil_div(U256::from(10u64), U256::from(3u64)),
            U256::from(4u64)
        );
        assert_eq!(
            ceil_div(U256::from(9u64), U256::from(3u64)),
            U256::from(3u64)
        );
        assert_eq!(
            ceil_div(U256::from(1u64), U256::from(1u64)),
            U256::from(1u64)
        );
    }

    #[test]
    fn get_x0_balanced() {
        // L=2, collateral=1 ETH at $2000, debt=$1000
        let leverage = U256::from(2u64) * WAD;
        let denom = U256::from(2u64) * leverage - WAD;
        let lev_ratio = leverage * leverage * WAD / denom / denom;

        let p_oracle = U256::from(2000u64) * WAD;
        let collateral = WAD; // 1 ETH
        let collateral_precision = U256::from(1u64); // 18 decimals
        let debt = U256::from(1000u64) * WAD;

        let x0 = get_x0(
            p_oracle,
            collateral,
            debt,
            collateral_precision,
            lev_ratio,
            false,
            U256::ZERO,
            U256::ZERO,
        )
        .unwrap();
        assert!(x0 > debt, "x0 should be > debt, got {x0}");
    }

    #[test]
    fn get_x0_negative_discriminant_returns_none() {
        let leverage = U256::from(2u64) * WAD;
        let denom = U256::from(2u64) * leverage - WAD;
        let lev_ratio = leverage * leverage * WAD / denom / denom;

        let p_oracle = U256::from(2000u64) * WAD;
        let collateral = WAD;
        let collateral_precision = U256::from(1u64);
        // debt too large → negative discriminant
        let debt = U256::from(5000u64) * WAD;

        assert!(get_x0(
            p_oracle,
            collateral,
            debt,
            collateral_precision,
            lev_ratio,
            false,
            U256::ZERO,
            U256::ZERO
        )
        .is_none());
    }

    #[test]
    fn compute_rate_mul_accrues() {
        let stored = WAD;
        let rate = WAD / U256::from(365u64 * 86400u64); // ~100% APR
        let rate_time = U256::from(1000u64);
        let now = U256::from(1000u64 + 86400u64); // 1 day later

        let rm = compute_rate_mul(stored, rate, rate_time, now);
        // Should be ~1.00274 * 1e18
        assert!(rm > WAD);
        assert!(rm < WAD + WAD / U256::from(100u64)); // < 1% for 1 day
    }

    #[test]
    fn compute_rate_mul_no_time_elapsed() {
        let stored = U256::from(1_050_000_000_000_000_000u128); // 1.05
        let rm = compute_rate_mul(stored, WAD, U256::from(100u64), U256::from(100u64));
        assert_eq!(rm, stored);
    }
}
