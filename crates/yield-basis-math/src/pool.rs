//! `YieldBasisPool` — high-level LEVAMM pool interface.
//!
//! Token indices: `0` = stablecoin, `1` = collateral.

use alloy_primitives::U256;

use crate::constants::WAD;
use crate::swap;

/// Error returned by pool methods.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PoolError {
    InvalidIndex,
    InvalidParams,
    MathError,
}

impl std::fmt::Display for PoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidIndex => f.write_str("invalid token index: must be 0 or 1"),
            Self::InvalidParams => f.write_str("invalid pool parameters"),
            Self::MathError => f.write_str("math error during computation"),
        }
    }
}

impl std::error::Error for PoolError {}

/// Yield Basis LEVAMM pool state snapshot.
#[derive(Debug, Clone)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct YieldBasisPool {
    /// `L * 1e18` (e.g. 2e18 for 2x leverage)
    pub leverage: U256,
    /// `L^2 * 1e18 / (2L - 1)^2`
    pub lev_ratio: U256,
    /// `10^(18 - collateral_decimals)`
    pub collateral_precision: U256,
    pub fee: U256,
    /// Collateral amount in AMM (native decimals).
    pub collateral_amount: U256,
    /// Current debt (after rate_mul accrual, 18 decimals).
    pub debt: U256,
    /// Oracle price in WAD.
    pub p_oracle: U256,
}

impl YieldBasisPool {
    /// Returns `Err(InvalidParams)` if leverage <= 1e18 or precision is zero.
    pub fn new(
        leverage: U256,
        lev_ratio: U256,
        collateral_precision: U256,
        fee: U256,
        collateral_amount: U256,
        debt: U256,
        p_oracle: U256,
    ) -> Result<Self, PoolError> {
        if leverage <= WAD || collateral_precision.is_zero() || lev_ratio.is_zero() {
            return Err(PoolError::InvalidParams);
        }
        Ok(Self {
            leverage,
            lev_ratio,
            collateral_precision,
            fee,
            collateral_amount,
            debt,
            p_oracle,
        })
    }

    /// `AMM.vy::get_dy(i, j, in_amount)`. Token indices: 0 = stablecoin, 1 = collateral.
    pub fn get_amount_out(&self, i: usize, j: usize, dx: U256) -> Result<U256, PoolError> {
        if !((i == 0 && j == 1) || (i == 1 && j == 0)) {
            return Err(PoolError::InvalidIndex);
        }
        swap::calc_swap_out(
            i,
            dx,
            self.p_oracle,
            self.collateral_amount,
            self.debt,
            self.collateral_precision,
            self.lev_ratio,
            self.fee,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::WAD;

    fn test_pool() -> YieldBasisPool {
        let leverage = U256::from(2u64) * WAD;
        let denom = U256::from(2u64) * leverage - WAD;
        let lev_ratio = leverage * leverage * WAD / denom / denom;

        YieldBasisPool::new(
            leverage,
            lev_ratio,
            U256::from(1u64),
            U256::from(3_000_000_000_000_000u128), // 0.3%
            WAD * U256::from(100u64),              // 100 collateral
            U256::from(100_000u64) * WAD,          // $100k debt
            U256::from(2000u64) * WAD,             // $2000 oracle
        )
        .unwrap()
    }

    #[test]
    fn invalid_index() {
        let pool = test_pool();
        assert_eq!(
            pool.get_amount_out(0, 0, WAD).unwrap_err(),
            PoolError::InvalidIndex
        );
    }

    #[test]
    fn zero_returns_zero() {
        let pool = test_pool();
        assert_eq!(pool.get_amount_out(0, 1, U256::ZERO).unwrap(), U256::ZERO);
    }

    #[test]
    fn buy_collateral() {
        let pool = test_pool();
        let dy = pool
            .get_amount_out(0, 1, U256::from(1000u64) * WAD)
            .unwrap();
        assert!(dy > U256::ZERO);
    }

    #[test]
    fn sell_collateral() {
        let pool = test_pool();
        let dy = pool.get_amount_out(1, 0, WAD).unwrap();
        assert!(dy > U256::ZERO);
    }

    #[test]
    fn invalid_params() {
        assert_eq!(
            YieldBasisPool::new(
                WAD, // leverage = 1e18, not > 1e18
                WAD,
                U256::from(1u64),
                U256::ZERO,
                U256::ZERO,
                U256::ZERO,
                WAD,
            )
            .unwrap_err(),
            PoolError::InvalidParams
        );
    }
}
