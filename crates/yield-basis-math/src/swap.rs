//! Swap math for Yield Basis LEVAMM.
//!
//! Ported from `AMM.vy` L247-271: `get_dy`
//!
//! Source: <https://github.com/yield-basis/yb-core/blob/main/contracts/AMM.vy>

use alloy_primitives::U256;

use crate::core::{ceil_div, get_x0};
use crate::pool::PoolError;

const WAD: U256 = U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);

/// Compute swap output for the LEVAMM.
///
/// `AMM.vy` L247-271: `get_dy(i, j, in_amount)`
///
/// Constant-product AMM where:
/// - x = `x0 - debt` (available stablecoins)
/// - y = `collateral`
/// - k = x * y (invariant)
///
/// `i=0`: buy collateral (stablecoin in, collateral out)
/// `i=1`: sell collateral (collateral in, stablecoin out)
#[allow(clippy::too_many_arguments)]
pub fn calc_swap_out(
    i: usize,
    in_amount: U256,
    p_oracle: U256,
    collateral: U256,
    debt: U256,
    collateral_precision: U256,
    lev_ratio: U256,
    fee: U256,
) -> Result<U256, PoolError> {
    if in_amount.is_zero() {
        return Ok(U256::ZERO);
    }

    // x0 = get_x0(p_oracle, collateral, debt, safe_limits=false)
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
    .ok_or(PoolError::MathError)?;

    // x_initial = x0 - debt
    if x0 <= debt {
        return Err(PoolError::MathError);
    }
    let x_initial = x0 - debt;

    let out = if i == 0 {
        // Buy collateral: stablecoin in
        if in_amount > debt {
            return Err(PoolError::MathError); // "Amount too large"
        }
        let x = x_initial + in_amount;
        // y = ceil(x_initial * collateral / x)
        let y = ceil_div(x_initial * collateral, x);
        if collateral <= y {
            return Ok(U256::ZERO);
        }
        // (collateral - y) * (1e18 - fee) / 1e18
        (collateral - y) * (WAD - fee) / WAD
    } else {
        // Sell collateral: collateral in
        let y = collateral + in_amount;
        // x = ceil(x_initial * collateral / y)
        let x = ceil_div(x_initial * collateral, y);
        if x_initial <= x {
            return Ok(U256::ZERO);
        }
        // (x_initial - x) * (1e18 - fee) / 1e18
        (x_initial - x) * (WAD - fee) / WAD
    };

    if out.is_zero() {
        return Err(PoolError::MathError);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_params() -> (U256, U256, U256, U256, U256, U256) {
        let leverage = U256::from(2u64) * WAD;
        let denom = U256::from(2u64) * leverage - WAD;
        let lev_ratio = leverage * leverage * WAD / denom / denom;
        let p_oracle = U256::from(2000u64) * WAD;
        let collateral = WAD * U256::from(100u64); // 100 ETH
        let debt = U256::from(100_000u64) * WAD; // $100k
        let collateral_precision = U256::from(1u64);
        let fee = U256::from(3_000_000_000_000_000u128); // 0.3%

        (
            p_oracle,
            collateral,
            debt,
            collateral_precision,
            lev_ratio,
            fee,
        )
    }

    #[test]
    fn buy_collateral_produces_output() {
        let (p_oracle, collateral, debt, cp, lr, fee) = test_params();
        let dx = U256::from(1000u64) * WAD; // $1000
        let dy = calc_swap_out(0, dx, p_oracle, collateral, debt, cp, lr, fee).unwrap();
        assert!(dy > U256::ZERO, "should get collateral, got {dy}");
    }

    #[test]
    fn sell_collateral_produces_output() {
        let (p_oracle, collateral, debt, cp, lr, fee) = test_params();
        let dx = WAD; // 1 ETH
        let dy = calc_swap_out(1, dx, p_oracle, collateral, debt, cp, lr, fee).unwrap();
        assert!(dy > U256::ZERO, "should get stablecoin, got {dy}");
    }

    #[test]
    fn zero_input_returns_zero() {
        let (p_oracle, collateral, debt, cp, lr, fee) = test_params();
        let dy = calc_swap_out(0, U256::ZERO, p_oracle, collateral, debt, cp, lr, fee).unwrap();
        assert_eq!(dy, U256::ZERO);
    }

    #[test]
    fn larger_input_more_output() {
        let (p_oracle, collateral, debt, cp, lr, fee) = test_params();
        let small = U256::from(100u64) * WAD;
        let large = U256::from(10_000u64) * WAD;
        let dy_small = calc_swap_out(0, small, p_oracle, collateral, debt, cp, lr, fee).unwrap();
        let dy_large = calc_swap_out(0, large, p_oracle, collateral, debt, cp, lr, fee).unwrap();
        assert!(dy_large > dy_small, "{dy_large} should > {dy_small}");
    }

    #[test]
    fn fee_reduces_output() {
        let (p_oracle, collateral, debt, cp, lr, _) = test_params();
        let dx = U256::from(1000u64) * WAD;
        let dy_no_fee =
            calc_swap_out(0, dx, p_oracle, collateral, debt, cp, lr, U256::ZERO).unwrap();
        let dy_with_fee = calc_swap_out(
            0,
            dx,
            p_oracle,
            collateral,
            debt,
            cp,
            lr,
            WAD / U256::from(100u64),
        )
        .unwrap();
        assert!(dy_no_fee > dy_with_fee, "fee should reduce output");
    }

    #[test]
    fn amount_too_large_returns_error() {
        let (p_oracle, collateral, debt, cp, lr, fee) = test_params();
        // Try to buy more than debt allows
        let dx = debt + WAD;
        assert!(calc_swap_out(0, dx, p_oracle, collateral, debt, cp, lr, fee).is_err());
    }
}
