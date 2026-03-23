use alloy_primitives::Address;
use std::collections::HashMap;

use crate::CurveVariant;

/// Supported chains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Chain {
    Ethereum,
}

/// Types of deployment events a factory can emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployEvent {
    /// `PlainPoolDeployed` — coins from event, pool address from call return data.
    PlainPoolDeployed,
    /// `MetaPoolDeployed` — coin + base_pool from event, pool address from call return data.
    MetaPoolDeployed,
    /// `TwocryptoPoolDeployed` — pool address + coins + packed params from event.
    TwocryptoPoolDeployed,
    /// `TricryptoPoolDeployed` — pool address + coins + packed params from event.
    TricryptoPoolDeployed,
    /// `CryptoPoolDeployed` — coins + A + gamma + fees from event, pool address from call return data.
    CryptoPoolDeployed,
}

/// A Curve factory contract that deploys pools.
#[derive(Debug, Clone)]
pub struct Factory {
    /// Factory contract address.
    pub address: Address,
    /// The variant of pools this factory deploys.
    /// For TwoCrypto-NG, the actual variant depends on the MATH address in the deploy event.
    pub default_variant: CurveVariant,
    /// Which deployment events this factory emits.
    /// Some factories emit multiple event types (e.g. StableSwapNG emits both
    /// PlainPoolDeployed and MetaPoolDeployed).
    pub deploy_events: Vec<(DeployEvent, CurveVariant)>,
}

/// Per-chain configuration: factory addresses and MATH lookup table.
#[derive(Debug, Clone)]
pub struct ChainConfig {
    /// NG factory contracts that emit deployment events.
    pub factories: Vec<Factory>,
    /// MATH contract address → CurveVariant for TwoCrypto-NG pools.
    /// The deploy event contains the MATH address, which determines
    /// whether the pool uses CryptoSwap (TwoCryptoNG) or StableSwap (TwoCryptoStable) math.
    pub math_to_variant: HashMap<Address, CurveVariant>,
    /// First block where any Curve pool was deployed on this chain.
    pub initial_block: u64,
}

/// Get the chain configuration (factory addresses, MATH lookup, initial block).
pub fn factories(chain: Chain) -> ChainConfig {
    match chain {
        Chain::Ethereum => ethereum_config(),
    }
}

fn ethereum_config() -> ChainConfig {
    let mut math_to_variant = HashMap::new();
    // v2.0.0 — CryptoSwap with gamma
    math_to_variant.insert(
        "0x2005995a71243be9FB995DaB4742327dc76564Df".parse().unwrap(),
        CurveVariant::TwoCryptoNG,
    );
    // v2.1.0 — CryptoSwap with gamma (same math, different compiler)
    math_to_variant.insert(
        "0x1Fd8Af16DC4BEBd950521308D55d0543b6cDF4A1".parse().unwrap(),
        CurveVariant::TwoCryptoNG,
    );
    // v0.1.0 — StableSwap (gamma ignored)
    math_to_variant.insert(
        "0x79839c2D74531A8222C0F555865aAc1834e82e51".parse().unwrap(),
        CurveVariant::TwoCryptoStable,
    );

    ChainConfig {
        factories: vec![
            // ── NG factories ────────────────────────────────────────────
            Factory {
                address: "0x6A8cbed756804B16E05E741eDaBd5cB544AE21bf".parse().unwrap(),
                default_variant: CurveVariant::StableSwapNG,
                deploy_events: vec![
                    (DeployEvent::PlainPoolDeployed, CurveVariant::StableSwapNG),
                    (DeployEvent::MetaPoolDeployed, CurveVariant::StableSwapMeta),
                ],
            },
            Factory {
                address: "0x98EE851a00abeE0d95D08cF4CA2BdCE32aeaAF7F".parse().unwrap(),
                default_variant: CurveVariant::TwoCryptoNG, // resolved by MATH lookup
                deploy_events: vec![
                    (DeployEvent::TwocryptoPoolDeployed, CurveVariant::TwoCryptoNG),
                ],
            },
            Factory {
                address: "0x0c0e5f2fF0ff18a3be9b835635039256dC4B4963".parse().unwrap(),
                default_variant: CurveVariant::TriCryptoNG,
                deploy_events: vec![
                    (DeployEvent::TricryptoPoolDeployed, CurveVariant::TriCryptoNG),
                ],
            },
            // ── Legacy factories ────────────────────────────────────────
            Factory {
                address: "0xF18056Bbd320E96A48e3Fbf8bC061322531aac99".parse().unwrap(),
                default_variant: CurveVariant::TwoCryptoV1,
                deploy_events: vec![
                    (DeployEvent::CryptoPoolDeployed, CurveVariant::TwoCryptoV1),
                ],
            },
            Factory {
                address: "0xB9fC157394Af804a3578134A6585C0dc9cc990d4".parse().unwrap(),
                default_variant: CurveVariant::StableSwapV2,
                deploy_events: vec![
                    (DeployEvent::PlainPoolDeployed, CurveVariant::StableSwapV2),
                    (DeployEvent::MetaPoolDeployed, CurveVariant::StableSwapMeta),
                ],
            },
            Factory {
                address: "0x4F8846Ae9380B90d2E71D5e3D042dff3E7ebb40d".parse().unwrap(),
                default_variant: CurveVariant::StableSwapV2,
                deploy_events: vec![
                    (DeployEvent::PlainPoolDeployed, CurveVariant::StableSwapV2),
                ],
            },
        ],
        math_to_variant,
        initial_block: 9_906_598, // first Curve pool on Ethereum (sUSD deploy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ethereum_has_6_factories() {
        let config = factories(Chain::Ethereum);
        assert_eq!(config.factories.len(), 6);
    }

    #[test]
    fn ethereum_math_lookup() {
        let config = factories(Chain::Ethereum);
        let v200: Address = "0x2005995a71243be9FB995DaB4742327dc76564Df".parse().unwrap();
        let v010: Address = "0x79839c2D74531A8222C0F555865aAc1834e82e51".parse().unwrap();
        assert_eq!(config.math_to_variant[&v200], CurveVariant::TwoCryptoNG);
        assert_eq!(config.math_to_variant[&v010], CurveVariant::TwoCryptoStable);
    }
}
