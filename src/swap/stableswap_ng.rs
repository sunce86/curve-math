//! Pool-level get_amount_out for StableSwapNG (plain + meta NG pools).
//!
//! -1 offset. Fee before denormalize. Dynamic fee with avg xp.

use alloy_primitives::U256;

use crate::core::stableswap_ng::{dynamic_fee, get_d, get_y, FEE_DENOMINATOR, PRECISION};

pub fn get_amount_out(
    balances: &[U256],
    rates: &[U256],
    amp: U256,
    fee: U256,
    offpeg_fee_multiplier: U256,
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

    // Vyper: fee = _dynamic_fee((xp[i]+x)/2, (xp[j]+y)/2, ...) * dy / FEE_DENOM
    let fee_rate = dynamic_fee(
        (xp[i] + x_new) / U256::from(2u64),
        (xp[j] + y_new) / U256::from(2u64),
        fee,
        offpeg_fee_multiplier,
    );
    let fee_amount = fee_rate * dy / U256::from(FEE_DENOMINATOR);

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
    use crate::core::stableswap_ng::A_PRECISION;

    const RATE_18: u128 = 1_000_000_000_000_000_000;

    #[test]
    fn basic_swap() {
        let b = U256::from(1_000_000_000_000_000_000_000_000u128);
        let out = get_amount_out(
            &[b, b],
            &[U256::from(RATE_18), U256::from(RATE_18)],
            U256::from(400u64 * A_PRECISION as u64),
            U256::from(4_000_000u64),
            U256::from(20_000_000_000u64),
            0,
            1,
            U256::from(1_000_000_000_000_000_000_000u128),
        )
        .expect("swap");
        assert!(out > U256::ZERO);
    }

    alloy::sol! {
        #[sol(rpc)]
        interface ICurvePoolNG {
            function get_dy(int128 i, int128 j, uint256 dx) external view returns (uint256);
            function balances(uint256 i) external view returns (uint256);
            function A() external view returns (uint256);
            function fee() external view returns (uint256);
            function offpeg_fee_multiplier() external view returns (uint256);
        }
    }

    #[tokio::test]
    #[ignore = "requires RPC_URL env var pointing to Ethereum mainnet"]
    async fn verify_usde_dai() {
        use alloy::providers::{Provider, ProviderBuilder};
        use std::str::FromStr;

        let rpc_url = std::env::var("RPC_URL").expect("RPC_URL must be set");
        let provider =
            ProviderBuilder::new().connect_http(rpc_url.parse().expect("invalid RPC_URL"));

        let pool_address =
            alloy_primitives::Address::from_str("0xF36a4BA50C603204c3FC6d2dA8b78A7b69CBC67d")
                .expect("valid");
        let curve = ICurvePoolNG::new(pool_address, &provider);

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
        let offpeg = curve
            .offpeg_fee_multiplier()
            .block(block)
            .call()
            .await
            .expect("offpeg");

        let balances = [r0, r1];
        let rates = [U256::from(RATE_18), U256::from(RATE_18)];
        let amp = raw_a * U256::from(A_PRECISION as u64);

        let dx = U256::from(1_000_000_000_000_000_000u128);
        let on_chain = curve
            .get_dy(0i128, 1i128, dx)
            .block(block)
            .call()
            .await
            .expect("get_dy");
        let ours =
            get_amount_out(&balances, &rates, amp, pool_fee, offpeg, 0, 1, dx).expect("ours");
        println!("  USDe→DAI: on_chain={on_chain}, ours={ours}");
        assert_eq!(ours, on_chain, "USDe→DAI mismatch");

        println!("StableSwapNG (USDe/DAI) verification passed!");
    }
}
