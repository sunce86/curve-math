use alloy_primitives::{Address, U256};

use crate::CurveVariant;

/// Information about a discovered Curve pool.
#[derive(Debug, Clone)]
pub struct PoolInfo {
    /// Pool contract address.
    pub address: Address,
    /// Detected variant.
    pub variant: CurveVariant,
    /// Token addresses in the pool.
    pub tokens: Vec<Address>,
    /// Initial A parameter (if available from deploy event).
    pub initial_a: Option<U256>,
    /// Initial fee (if available from deploy event).
    pub initial_fee: Option<U256>,
}

/// Parse a StableSwap-NG `PlainPoolDeployed` event.
///
/// Event signature: `PlainPoolDeployed(address[4] coins, uint256 A, uint256 fee, address deployer)`
///
/// # Arguments
/// * `pool_address` — the deployed pool address (from event or factory call)
/// * `coins` — coin addresses from the event (trailing zeros = unused slots)
/// * `a` — initial A parameter
/// * `fee` — initial fee
pub fn parse_stableswap_ng_deploy(
    pool_address: Address,
    coins: &[Address],
    a: U256,
    fee: U256,
) -> PoolInfo {
    let tokens: Vec<Address> = coins
        .iter()
        .copied()
        .filter(|addr| !addr.is_zero())
        .collect();

    PoolInfo {
        address: pool_address,
        variant: CurveVariant::StableSwapNG,
        tokens,
        initial_a: Some(a),
        initial_fee: Some(fee),
    }
}

/// Parse a TwoCrypto-NG `TwocryptoPoolDeployed` event.
///
/// The `math` address determines the variant:
/// - MATH v2.0.0 or v2.1.0 → `TwoCryptoNG` (CryptoSwap with gamma)
/// - MATH v0.1.0 → `TwoCryptoStable` (StableSwap, gamma ignored)
///
/// Use `ChainConfig::math_to_variant` to resolve the MATH address.
///
/// # Arguments
/// * `pool_address` — the deployed pool address
/// * `coins` — 2-element coin array
/// * `math` — MATH contract address (determines variant)
/// * `math_to_variant` — lookup function for MATH → variant
pub fn parse_twocrypto_ng_deploy(
    pool_address: Address,
    coins: &[Address; 2],
    math: Address,
    math_to_variant: impl Fn(Address) -> Option<CurveVariant>,
) -> PoolInfo {
    let variant = math_to_variant(math).unwrap_or(CurveVariant::TwoCryptoNG);

    PoolInfo {
        address: pool_address,
        variant,
        tokens: coins.to_vec(),
        initial_a: None,
        initial_fee: None,
    }
}

/// Parse a TriCrypto-NG `TricryptoPoolDeployed` event.
///
/// # Arguments
/// * `pool_address` — the deployed pool address
/// * `coins` — 3-element coin array
pub fn parse_tricrypto_ng_deploy(pool_address: Address, coins: &[Address; 3]) -> PoolInfo {
    PoolInfo {
        address: pool_address,
        variant: CurveVariant::TriCryptoNG,
        tokens: coins.to_vec(),
        initial_a: None,
        initial_fee: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stableswap_ng_filters_zero_coins() {
        let pool: Address = "0x1111111111111111111111111111111111111111"
            .parse()
            .unwrap();
        let coins = [
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
                .parse()
                .unwrap(),
            "0xdAC17F958D2ee523a2206206994597C13D831ec7"
                .parse()
                .unwrap(),
            Address::ZERO,
            Address::ZERO,
        ];
        let info = parse_stableswap_ng_deploy(pool, &coins, U256::from(500), U256::from(4000000));
        assert_eq!(info.tokens.len(), 2);
        assert_eq!(info.variant, CurveVariant::StableSwapNG);
    }

    #[test]
    fn twocrypto_detects_stable_variant() {
        let pool: Address = "0x2222222222222222222222222222222222222222"
            .parse()
            .unwrap();
        let coins: [Address; 2] = [
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
                .parse()
                .unwrap(),
            "0xdAC17F958D2ee523a2206206994597C13D831ec7"
                .parse()
                .unwrap(),
        ];
        let math_stable: Address = "0x79839c2D74531A8222C0F555865aAc1834e82e51"
            .parse()
            .unwrap();

        let info = parse_twocrypto_ng_deploy(pool, &coins, math_stable, |addr| {
            if addr == math_stable {
                Some(CurveVariant::TwoCryptoStable)
            } else {
                Some(CurveVariant::TwoCryptoNG)
            }
        });
        assert_eq!(info.variant, CurveVariant::TwoCryptoStable);
    }
}
