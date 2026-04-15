//! Build a `YieldBasisPool` from raw pool state.

use alloy_primitives::U256;
use yield_basis_math::core::{compute_debt, compute_rate_mul};
use yield_basis_math::pool::{PoolError, YieldBasisPool};

/// Raw LEVAMM pool state, data-source agnostic.
/// Populate from RPC, substream, database, or registry.
///
/// Immutable fields (`leverage`, `lev_ratio`, `collateral_precision`) are
/// set at deploy time. Mutable fields change per block.
#[derive(Debug, Clone)]
pub struct RawYieldBasisState {
    // -- Immutables (from constructor, never change) --
    /// `L * 1e18` (e.g. 2e18 for 2x leverage). Must be > 1e18.
    pub leverage: U256,
    /// `L^2 * 1e18 / (2L - 1)^2`, precomputed at deploy.
    pub lev_ratio: U256,
    /// `10^(18 - collateral_decimals)`
    pub collateral_precision: U256,

    // -- Mutable storage --
    pub fee: U256,
    /// Collateral in AMM (native decimals).
    pub collateral_amount: U256,
    /// Stored debt (before rate_mul adjustment).
    pub stored_debt: U256,
    /// Accumulated rate multiplier (starts at 1e18).
    pub rate_mul: U256,
    /// Interest rate per second.
    pub rate: U256,
    /// Timestamp of last rate update.
    pub rate_time: U256,

    // -- External --
    /// Oracle price in WAD.
    pub p_oracle: U256,
}

/// Build a `YieldBasisPool` from raw state. Pure, no I/O.
///
/// `block_timestamp` is needed to accrue interest on debt.
pub fn build_pool(
    state: &RawYieldBasisState,
    block_timestamp: U256,
) -> Result<YieldBasisPool, PoolError> {
    // Accrue interest: rate_mul = stored_rate_mul * (1 + rate * dt) / 1e18
    let current_rate_mul =
        compute_rate_mul(state.rate_mul, state.rate, state.rate_time, block_timestamp);

    // Current debt = stored_debt * current_rate_mul / 1e18
    let debt = compute_debt(state.stored_debt, current_rate_mul);

    YieldBasisPool::new(
        state.leverage,
        state.lev_ratio,
        state.collateral_precision,
        state.fee,
        state.collateral_amount,
        debt,
        state.p_oracle,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAD: U256 = U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);

    #[test]
    fn build_basic() {
        let leverage = U256::from(2u64) * WAD;
        let denom = U256::from(2u64) * leverage - WAD;
        let lev_ratio = leverage * leverage * WAD / denom / denom;

        let state = RawYieldBasisState {
            leverage,
            lev_ratio,
            collateral_precision: U256::from(1u64),
            fee: U256::from(3_000_000_000_000_000u128),
            collateral_amount: WAD * U256::from(100u64),
            stored_debt: U256::from(100_000u64) * WAD,
            rate_mul: WAD,
            rate: U256::ZERO,
            rate_time: U256::from(1000u64),
            p_oracle: U256::from(2000u64) * WAD,
        };

        let pool = build_pool(&state, U256::from(1000u64)).unwrap();
        let dy = pool
            .get_amount_out(0, 1, U256::from(1000u64) * WAD)
            .unwrap();
        assert!(dy > U256::ZERO);
    }

    #[test]
    fn build_with_interest_accrual() {
        let leverage = U256::from(2u64) * WAD;
        let denom = U256::from(2u64) * leverage - WAD;
        let lev_ratio = leverage * leverage * WAD / denom / denom;

        let rate = WAD / U256::from(365u64 * 86400u64); // ~100% APR
        let state = RawYieldBasisState {
            leverage,
            lev_ratio,
            collateral_precision: U256::from(1u64),
            fee: U256::ZERO,
            collateral_amount: WAD * U256::from(100u64),
            stored_debt: U256::from(100_000u64) * WAD,
            rate_mul: WAD,
            rate,
            rate_time: U256::from(1000u64),
            p_oracle: U256::from(2000u64) * WAD,
        };

        // Build at t=1000 (no accrual)
        let pool_t0 = build_pool(&state, U256::from(1000u64)).unwrap();
        // Build at t=1000+86400 (1 day of interest)
        let pool_t1 = build_pool(&state, U256::from(1000u64 + 86400u64)).unwrap();

        // Interest accrual changes debt → different pool state → different output.
        // Just verify both produce valid (nonzero) outputs and differ.
        let dx = U256::from(1000u64) * WAD;
        let dy_t0 = pool_t0.get_amount_out(0, 1, dx).unwrap();
        let dy_t1 = pool_t1.get_amount_out(0, 1, dx).unwrap();
        assert!(dy_t0 > U256::ZERO);
        assert!(dy_t1 > U256::ZERO);
        assert!(dy_t0 != dy_t1, "interest should change output");
    }

    #[test]
    fn build_rejects_invalid_leverage() {
        let state = RawYieldBasisState {
            leverage: WAD, // L=1, invalid
            lev_ratio: WAD,
            collateral_precision: U256::from(1u64),
            fee: U256::ZERO,
            collateral_amount: U256::ZERO,
            stored_debt: U256::ZERO,
            rate_mul: WAD,
            rate: U256::ZERO,
            rate_time: U256::ZERO,
            p_oracle: WAD,
        };
        assert!(build_pool(&state, U256::ZERO).is_err());
    }
}
