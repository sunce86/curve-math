//! Pool-level get_amount_out for StableSwapV1 (3pool, ren, sbtc, hbtc).
//!
//! -1 offset. Denorm FIRST, then fee.

use alloy_primitives::U256;

use crate::core::stableswap_v1::{get_d, get_y, FEE_DENOMINATOR, PRECISION};

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
    // -1 offset. Denorm FIRST, then fee.
    let dy = (xp[j] - y_new - U256::from(1)) * precision / rates[j];
    let fee_amount = fee * dy / U256::from(FEE_DENOMINATOR);
    let result = dy - fee_amount;
    if result.is_zero() {
        return None;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    alloy::sol! {
        #[sol(rpc)]
        interface ICurvePool {
            function get_dy(int128 i, int128 j, uint256 dx) external view returns (uint256);
            function balances(uint256 i) external view returns (uint256);
            function A() external view returns (uint256);
            function fee() external view returns (uint256);
        }
    }

    #[tokio::test]
    #[ignore = "requires RPC_URL env var pointing to Ethereum mainnet"]
    async fn verify_3pool() {
        use alloy::providers::{Provider, ProviderBuilder};
        use std::str::FromStr;
        let rpc_url = std::env::var("RPC_URL").expect("RPC_URL");
        let provider = ProviderBuilder::new().connect_http(rpc_url.parse().expect("url"));
        let addr =
            alloy_primitives::Address::from_str("0xbEbc44782C7dB0a1A60Cb6fe97d0b483032FF1C7")
                .expect("addr");
        let curve = ICurvePool::new(addr, &provider);
        let bn = provider.get_block_number().await.expect("bn");
        let block = alloy::eips::BlockId::number(bn);

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
        let r2 = curve
            .balances(U256::from(2))
            .block(block)
            .call()
            .await
            .expect("b2");
        let raw_a = curve.A().block(block).call().await.expect("A");
        let pool_fee = curve.fee().block(block).call().await.expect("fee");

        let rate18 = U256::from(1_000_000_000_000_000_000u128);
        let rate6 = U256::from(1_000_000_000_000_000_000_000_000_000_000u128);

        for (i, j, amount, label) in [
            (
                0,
                1,
                U256::from(1_000_000_000_000_000_000_000u128),
                "DAI→USDC",
            ),
            (1, 0, U256::from(1_000_000_000u128), "USDC→DAI"),
            (
                0,
                2,
                U256::from(1_000_000_000_000_000_000_000u128),
                "DAI→USDT",
            ),
            (2, 1, U256::from(1_000_000_000u128), "USDT→USDC"),
        ] {
            let on_chain = curve
                .get_dy(i as i128, j as i128, amount)
                .block(block)
                .call()
                .await
                .expect("dy");
            let ours = get_amount_out(
                &[r0, r1, r2],
                &[rate18, rate6, rate6],
                raw_a,
                pool_fee,
                i as usize,
                j as usize,
                amount,
            )
            .expect("ours");
            println!("  {label}: on_chain={on_chain}, ours={ours}");
            assert_eq!(ours, on_chain, "{label} mismatch");
        }
        println!("StableSwapV1 (3pool) passed!");
    }
}
