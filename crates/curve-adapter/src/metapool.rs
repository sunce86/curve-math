use alloy_primitives::Address;

use crate::Chain;

/// Resolve the LP token address for a base pool used in metapools.
///
/// Metapools pair a meta coin with the LP token of a base pool (e.g. 3CRV for 3pool).
/// The LP token address differs from the base pool contract address for legacy pools
/// (which have separate LP token contracts). NG pools are their own LP token.
///
/// Returns the LP token address for known base pools, or the base pool address itself
/// as fallback (correct for NG pools where pool == LP token).
pub fn resolve_base_pool_lp_token(chain: Chain, base_pool: Address) -> Address {
    match chain {
        Chain::Ethereum => resolve_ethereum(base_pool),
    }
}

fn resolve_ethereum(base_pool: Address) -> Address {
    match base_pool {
        // 3pool → 3CRV
        addr if addr == "0xbEbc44782C7dB0a1A60Cb6fe97d0b483032FF1C7".parse::<Address>().unwrap() => {
            "0x6c3F90f043a72FA612cbac8115EE7e52BDe6E490".parse().unwrap()
        }
        // FRAX/USDC → crvFRAX
        addr if addr == "0xDcEF968d416a41Cdac0ED8702fAC8128A64241A2".parse::<Address>().unwrap() => {
            "0x3175Df0976dFA876431C2E9eE6Bc45b65d3473CC".parse().unwrap()
        }
        // renBTC/wBTC/sBTC → crvRenWSBTC
        addr if addr == "0x7fC77b5c7614E1533320Ea6DDc2Eb61fa00A9714".parse::<Address>().unwrap() => {
            "0x075b1bb99792c9E1041bA13afEf80C91a1e70fB3".parse().unwrap()
        }
        // NG pools are their own LP token — base_pool address IS the LP token
        _ => base_pool,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_pool_lp_token() {
        let base: Address = "0xbEbc44782C7dB0a1A60Cb6fe97d0b483032FF1C7".parse().unwrap();
        let lp = resolve_base_pool_lp_token(Chain::Ethereum, base);
        let expected: Address = "0x6c3F90f043a72FA612cbac8115EE7e52BDe6E490".parse().unwrap();
        assert_eq!(lp, expected);
    }

    #[test]
    fn ng_pool_is_own_lp_token() {
        let ng_pool: Address = "0xF36a4BA50C603204c3FC6d2dA8b78A7b69CBC67d".parse().unwrap();
        let lp = resolve_base_pool_lp_token(Chain::Ethereum, ng_pool);
        assert_eq!(lp, ng_pool); // fallback: pool == LP token
    }
}
