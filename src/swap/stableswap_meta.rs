//! Pool-level get_amount_out for StableSwapMeta (GUSD/3CRV, HUSD, factory meta).
//!
//! -1 offset. Fee before denormalize. Static fee.
//! rates[1] (LP token) must be set to fresh virtual_price from base pool.

use alloy_primitives::U256;

use crate::core::stableswap_meta::{get_d, get_y, FEE_DENOMINATOR, PRECISION};

pub fn get_amount_out(
    balances: &[U256],
    rates: &[U256],
    amp: U256,
    fee: U256,
    i: usize,
    j: usize,
    dx: U256,
) -> Option<U256> {
    if dx.is_zero() {
        return None;
    }

    let precision = U256::from(PRECISION);

    let xp: Vec<U256> = balances
        .iter()
        .zip(rates.iter())
        .map(|(b, r)| *b * *r / precision)
        .collect();

    let d = get_d(&xp, amp)?;
    let x_new = xp[i] + dx * rates[i] / precision;
    let y_new = get_y(i, j, x_new, &xp, d, amp)?;

    if xp[j] <= y_new {
        return None;
    }

    // Vyper: dy = xp[j] - y - 1
    let dy = xp[j] - y_new - U256::from(1);

    // Vyper: fee = self.fee * dy / FEE_DENOMINATOR
    let fee_amount = fee * dy / U256::from(FEE_DENOMINATOR);

    // Vyper: return (dy - fee) * PRECISION / rates[j]
    let result = (dy - fee_amount) * precision / rates[j];
    if result.is_zero() {
        return None;
    }

    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::stableswap_meta::A_PRECISION;

    alloy::sol! {
        #[sol(rpc)]
        interface ICurvePool {
            function get_dy(int128 i, int128 j, uint256 dx) external view returns (uint256);
            function balances(uint256 i) external view returns (uint256);
            function A() external view returns (uint256);
            function fee() external view returns (uint256);
            function base_pool() external view returns (address);
            function base_virtual_price() external view returns (uint256);
            function base_cache_updated() external view returns (uint256);
        }
        #[sol(rpc)]
        interface IBasePool {
            function get_virtual_price() external view returns (uint256);
        }
    }

    #[tokio::test]
    #[ignore = "requires RPC_URL env var pointing to Ethereum mainnet"]
    async fn verify_gusd() {
        use alloy::providers::{Provider, ProviderBuilder};
        use std::str::FromStr;

        let rpc_url = std::env::var("RPC_URL").expect("RPC_URL must be set");
        let provider =
            ProviderBuilder::new().connect_http(rpc_url.parse().expect("invalid RPC_URL"));

        let pool_address =
            alloy_primitives::Address::from_str("0x4f062658EaAF2C1ccf8C8e36D6824CDf41167956")
                .expect("valid");
        let curve = ICurvePool::new(pool_address, &provider);

        let block_num = provider.get_block_number().await.expect("block");
        let block = alloy::eips::BlockId::number(block_num);

        let r0 = curve
            .balances(U256::from(0))
            .block(block)
            .call()
            .await
            .expect("b0");
        let r1 = curve
            .balances(U256::from(1))
            .block(block)
            .call()
            .await
            .expect("b1");
        let raw_a = curve.A().block(block).call().await.expect("A");
        let pool_fee = curve.fee().block(block).call().await.expect("fee");

        // Read virtual_price matching _vp_rate_ro() behavior
        let base_pool_addr = curve
            .base_pool()
            .block(block)
            .call()
            .await
            .expect("base_pool");
        let base_pool = IBasePool::new(base_pool_addr, &provider);
        let cached_vp = curve
            .base_virtual_price()
            .block(block)
            .call()
            .await
            .expect("vp");
        let cache_updated = curve
            .base_cache_updated()
            .block(block)
            .call()
            .await
            .expect("ts");
        let block_data = provider
            .get_block_by_number(block_num.into())
            .await
            .expect("block_data")
            .expect("block");
        let block_ts = U256::from(block_data.header.timestamp);
        let vp = if block_ts - cache_updated > U256::from(600u64) {
            base_pool
                .get_virtual_price()
                .block(block)
                .call()
                .await
                .expect("fresh_vp")
        } else {
            cached_vp
        };

        let balances = [r0, r1];
        // GUSD(2-dec): stored_rate = 10^34. 3CRV(18-dec): stored_rate = virtual_price.
        let rate_gusd = U256::from(10u64).pow(U256::from(34u64));
        let rates = [rate_gusd, vp];
        let amp = raw_a * U256::from(A_PRECISION as u64);

        let dx = U256::from(10000u64); // 100 GUSD (2-dec)
        let on_chain = curve
            .get_dy(0i128, 1i128, dx)
            .block(block)
            .call()
            .await
            .expect("get_dy");
        let ours = get_amount_out(&balances, &rates, amp, pool_fee, 0, 1, dx).expect("ours");
        println!("  GUSD→3CRV: on_chain={on_chain}, ours={ours}");
        assert_eq!(ours, on_chain, "GUSD→3CRV mismatch");

        println!("StableSwapMeta (GUSD) verification passed!");
    }
}
