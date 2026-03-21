//! Pool-level get_amount_out for StableSwapV2 (FRAX/USDC, stETH, factory plain).
//!
//! -1 offset. Fee before denormalize.

use alloy_primitives::U256;

use crate::core::stableswap_v2::{get_d, get_y, FEE_DENOMINATOR, PRECISION};

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

    let dy = xp[j] - y_new - U256::from(1);
    let fee_amount = fee * dy / U256::from(FEE_DENOMINATOR);
    let result = (dy - fee_amount) * precision / rates[j];

    if result.is_zero() {
        return None;
    }

    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::stableswap_v2::A_PRECISION;

    const RATE_18: u128 = 1_000_000_000_000_000_000;
    const RATE_6: u128 = 1_000_000_000_000_000_000_000_000_000_000;

    #[test]
    fn basic_swap() {
        let b = U256::from(1_000_000_000_000_000_000_000_000u128);
        let out = get_amount_out(
            &[b, b],
            &[U256::from(RATE_18), U256::from(RATE_18)],
            U256::from(200u64 * A_PRECISION as u64),
            U256::from(4_000_000u64),
            0,
            1,
            U256::from(1_000_000_000_000_000_000_000u128),
        )
        .expect("swap");

        let amount_in = U256::from(1_000_000_000_000_000_000_000u128);
        assert!(out < amount_in);
        assert!(out > amount_in * U256::from(999) / U256::from(1000));
    }

    #[test]
    fn zero_returns_none() {
        let b = U256::from(1_000_000_000_000_000_000_000_000u128);
        assert!(get_amount_out(
            &[b, b],
            &[U256::from(RATE_18), U256::from(RATE_18)],
            U256::from(200u64 * A_PRECISION as u64),
            U256::from(4_000_000u64),
            0,
            1,
            U256::ZERO,
        )
        .is_none());
    }

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
    async fn verify_frax_usdc() {
        use alloy::providers::{Provider, ProviderBuilder};
        use std::str::FromStr;

        let rpc_url = std::env::var("RPC_URL").expect("RPC_URL must be set");
        let provider =
            ProviderBuilder::new().connect_http(rpc_url.parse().expect("invalid RPC_URL"));

        let pool_address =
            alloy_primitives::Address::from_str("0xDcEF968d416a41Cdac0ED8702fAC8128A64241A2")
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

        let balances = [r0, r1];
        let rates = [U256::from(RATE_18), U256::from(RATE_6)];
        let amp = raw_a * U256::from(A_PRECISION as u64);

        for (amount, label) in [
            (U256::from(1_000_000_000_000_000_000u128), "1 FRAX"),
            (U256::from(1_000_000_000_000_000_000_000u128), "1k FRAX"),
            (U256::from(100_000_000_000_000_000_000_000u128), "100k FRAX"),
        ] {
            let on_chain = curve
                .get_dy(0i128, 1i128, amount)
                .block(block)
                .call()
                .await
                .expect("get_dy");
            let ours =
                get_amount_out(&balances, &rates, amp, pool_fee, 0, 1, amount).expect("ours");
            println!("  {label}: on_chain={on_chain}, ours={ours}");
            assert_eq!(ours, on_chain, "FRAX→USDC mismatch for {label}");
        }

        for (amount, label) in [
            (U256::from(1_000_000u128), "1 USDC"),
            (U256::from(1_000_000_000u128), "1k USDC"),
            (U256::from(100_000_000_000u128), "100k USDC"),
        ] {
            let on_chain = curve
                .get_dy(1i128, 0i128, amount)
                .block(block)
                .call()
                .await
                .expect("get_dy");
            let ours =
                get_amount_out(&balances, &rates, amp, pool_fee, 1, 0, amount).expect("ours");
            println!("  {label}: on_chain={on_chain}, ours={ours}");
            assert_eq!(ours, on_chain, "USDC→FRAX mismatch for {label}");
        }

        println!("StableSwapV2 (FRAX/USDC) verification passed!");
    }
}
