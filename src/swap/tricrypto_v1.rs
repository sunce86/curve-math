//! Pool-level get_amount_out for TriCryptoV1 (tricrypto2: USDT/WBTC/WETH).

use alloy_primitives::U256;

use crate::core::tricrypto_v1::{crypto_fee, newton_y_3, FEE_DENOMINATOR, WAD};

pub fn get_amount_out(
    balances: &[U256; 3],
    precisions: &[U256; 3],
    price_scale: &[U256; 2],
    d: U256,
    ann: U256,
    gamma: U256,
    mid_fee: U256,
    out_fee: U256,
    fee_gamma: U256,
    i: usize,
    j: usize,
    dx: U256,
) -> Option<U256> {
    if dx.is_zero() {
        return None;
    }

    let wad = U256::from(WAD);

    // Vyper: xp = balances; xp[i] += dx
    let mut bal = *balances;
    bal[i] += dx;

    // Vyper: xp[0] *= precisions[0]; xp[k] = xp[k] * price_scale[k-1] * precisions[k] / PRECISION
    let xp: [U256; 3] = [
        bal[0] * precisions[0],
        bal[1] * price_scale[0] * precisions[1] / wad,
        bal[2] * price_scale[1] * precisions[2] / wad,
    ];

    // Vyper: y = newton_y(A, gamma, xp, D, j)
    let y = newton_y_3(ann, gamma, xp, d, j)?;

    if xp[j] <= y {
        return None;
    }

    // Vyper: dy = xp[j] - y - 1
    let mut dy = xp[j] - y - U256::from(1);

    // Vyper: xp[j] = y (for fee calc)
    let mut xp_after = xp;
    xp_after[j] = y;

    // Vyper: if j > 0: dy = dy * PRECISION / price_scale[j-1]
    if j > 0 {
        dy = dy * wad / price_scale[j - 1];
    }
    // Vyper: dy /= precisions[j]
    dy = dy / precisions[j];

    // Vyper: dy -= fee_calc(xp) * dy / 10**10
    let fee = crypto_fee(&xp_after, mid_fee, out_fee, fee_gamma)?;
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
        interface ITriCryptoPool {
            function get_dy(uint256 i, uint256 j, uint256 dx) external view returns (uint256);
            function balances(uint256 i) external view returns (uint256);
            function A() external view returns (uint256);
            function gamma() external view returns (uint256);
            function D() external view returns (uint256);
            function price_scale(uint256 i) external view returns (uint256);
            function mid_fee() external view returns (uint256);
            function out_fee() external view returns (uint256);
            function fee_gamma() external view returns (uint256);
        }
    }

    #[tokio::test]
    #[ignore = "requires RPC_URL env var pointing to Ethereum mainnet"]
    async fn verify_tricrypto2() {
        use alloy::providers::{Provider, ProviderBuilder};
        use std::str::FromStr;

        let rpc_url = std::env::var("RPC_URL").expect("RPC_URL must be set");
        let provider =
            ProviderBuilder::new().connect_http(rpc_url.parse().expect("invalid RPC_URL"));

        let pool_address =
            alloy_primitives::Address::from_str("0xD51a44d3FaE010294C616388b506AcdA1bfAAE46")
                .expect("valid");
        let curve = ITriCryptoPool::new(pool_address, &provider);

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
        let r2 = curve
            .balances(U256::from(2))
            .block(block)
            .call()
            .await
            .expect("b2");
        let a = curve.A().block(block).call().await.expect("A");
        let gamma = curve.gamma().block(block).call().await.expect("gamma");
        let d = curve.D().block(block).call().await.expect("D");
        let ps0 = curve
            .price_scale(U256::from(0))
            .block(block)
            .call()
            .await
            .expect("ps0");
        let ps1 = curve
            .price_scale(U256::from(1))
            .block(block)
            .call()
            .await
            .expect("ps1");
        let mid_fee = curve.mid_fee().block(block).call().await.expect("mid_fee");
        let out_fee = curve.out_fee().block(block).call().await.expect("out_fee");
        let fg = curve
            .fee_gamma()
            .block(block)
            .call()
            .await
            .expect("fee_gamma");

        // USDT=6dec, WBTC=8dec, WETH=18dec
        let balances = [r0, r1, r2];
        let precisions = [
            U256::from(1_000_000_000_000u64),
            U256::from(10_000_000_000u64),
            U256::from(1u64),
        ];
        let price_scale = [ps0, ps1];

        let dx = U256::from(1_000_000_000u128); // 1000 USDT
        let on_chain = curve
            .get_dy(U256::from(0), U256::from(1), dx)
            .block(block)
            .call()
            .await
            .expect("get_dy");
        let ours = get_amount_out(
            &balances,
            &precisions,
            &price_scale,
            d,
            a,
            gamma,
            mid_fee,
            out_fee,
            fg,
            0,
            1,
            dx,
        )
        .expect("ours");
        println!("  USDT→WBTC: on_chain={on_chain}, ours={ours}");
        assert_eq!(ours, on_chain, "USDT→WBTC mismatch");

        println!("TriCryptoV1 (tricrypto2) verification passed!");
    }
}
