use alloy_primitives::Address;

use crate::factories::Chain;
use crate::CurveVariant;

/// A legacy (pre-factory) Curve pool.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct LegacyPool {
    /// Pool contract address.
    pub address: Address,
    /// Pool variant.
    pub variant: CurveVariant,
    /// Token addresses in the pool.
    pub tokens: Vec<Address>,
}

#[derive(serde::Deserialize)]
struct LegacyRegistry {
    pools: Vec<LegacyPool>,
}

/// Get the complete list of legacy pools for a chain.
///
/// Legacy pools are pre-factory pools that cannot be discovered from events.
/// This list is static — no new legacy pools are deployed.
///
/// Source: Curve MetaRegistry + on-chain variant detection probes.
pub fn legacy_pools(chain: Chain) -> Vec<LegacyPool> {
    let toml_str = match chain {
        Chain::Ethereum => include_str!("../data/legacy_ethereum.toml"),
    };

    let registry: LegacyRegistry =
        toml::from_str(toml_str).expect("embedded legacy TOML should be valid");
    registry.pools
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ethereum_legacy_pools_load() {
        let pools = legacy_pools(Chain::Ethereum);
        assert!(!pools.is_empty(), "should have at least some legacy pools");

        // 3pool should be in there
        let three_pool: Address = "0xbEbc44782C7dB0a1A60Cb6fe97d0b483032FF1C7"
            .parse()
            .unwrap();
        let found = pools.iter().find(|p| p.address == three_pool);
        assert!(found.is_some(), "3pool should be in legacy registry");
        assert_eq!(found.unwrap().variant, CurveVariant::StableSwapV1);
        assert_eq!(found.unwrap().tokens.len(), 3);
    }
}
