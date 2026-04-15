//! Protocol constants for Yield Basis LEVAMM.

use alloy_primitives::U256;

/// 1e18
pub const WAD: U256 = U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);

/// Max fee: 10% (from AMM.vy `MAX_FEE`)
pub const MAX_FEE: U256 = U256::from_limbs([100_000_000_000_000_000, 0, 0, 0]);
