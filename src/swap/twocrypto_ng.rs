//! Pool-level get_amount_out for TwoCryptoNG (crvUSD/FXN, Cardano cubic solver).

use alloy_primitives::U256;

use crate::core::twocrypto_ng::{crypto_fee, get_y_2_ng, FEE_DENOMINATOR, WAD};

pub fn get_amount_out(
    balances: &[U256; 2],
    precisions: &[U256; 2],
    price_scale: U256,
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
    let price_scale_local = price_scale * precisions[1];

    let mut bal = *balances;
    bal[i] += dx;
    let xp: [U256; 2] = [bal[0] * precisions[0], bal[1] * price_scale_local / wad];

    // NG uses Cardano cubic solver
    let (y, _k0) = get_y_2_ng(ann, gamma, xp, d, j)?;

    if xp[j] <= y {
        return None;
    }

    let dy = xp[j] - y - U256::from(1);
    let xp_after: [U256; 2] = if j == 0 { [y, xp[1]] } else { [xp[0], y] };

    let dy_native = if j > 0 {
        dy * wad / price_scale_local
    } else {
        dy / precisions[0]
    };

    let fee = crypto_fee(&xp_after, mid_fee, out_fee, fee_gamma)?;
    let fee_amount = fee * dy_native / U256::from(FEE_DENOMINATOR);
    let result = dy_native - fee_amount;

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
        interface ICryptoPool {
            function get_dy(uint256 i, uint256 j, uint256 dx) external view returns (uint256);
            function balances(uint256 i) external view returns (uint256);
            function A() external view returns (uint256);
            function gamma() external view returns (uint256);
            function D() external view returns (uint256);
            function price_scale() external view returns (uint256);
            function mid_fee() external view returns (uint256);
            function out_fee() external view returns (uint256);
            function fee_gamma() external view returns (uint256);
        }
    }

    #[tokio::test]
    #[ignore = "requires RPC_URL env var pointing to Ethereum mainnet"]
    async fn verify_crvusd_fxn() {
        use alloy::providers::{Provider, ProviderBuilder};
        use std::str::FromStr;

        let rpc_url = std::env::var("RPC_URL").expect("RPC_URL must be set");
        let provider =
            ProviderBuilder::new().connect_http(rpc_url.parse().expect("invalid RPC_URL"));

        let pool_address =
            alloy_primitives::Address::from_str("0xfb8b95Fb2296a0Ad4b6b1419fdAA5AA5F13e4009")
                .expect("valid");
        let curve = ICryptoPool::new(pool_address, &provider);

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
        let a = curve.A().block(block).call().await.expect("A");
        let gamma = curve.gamma().block(block).call().await.expect("gamma");
        let d = curve.D().block(block).call().await.expect("D");
        let ps = curve.price_scale().block(block).call().await.expect("ps");
        let mid_fee = curve.mid_fee().block(block).call().await.expect("mid_fee");
        let out_fee = curve.out_fee().block(block).call().await.expect("out_fee");
        let fg = curve
            .fee_gamma()
            .block(block)
            .call()
            .await
            .expect("fee_gamma");

        let balances = [r0, r1];
        let precisions = [U256::from(1u64), U256::from(1u64)];

        for (i, j, amount, label) in [
            (
                0,
                1,
                U256::from(1_000_000_000_000_000_000u128),
                "1 crvUSD→FXN",
            ),
            (
                1,
                0,
                U256::from(1_000_000_000_000_000_000u128),
                "1 FXN→crvUSD",
            ),
        ] {
            let on_chain = curve
                .get_dy(U256::from(i), U256::from(j), amount)
                .block(block)
                .call()
                .await
                .expect("get_dy");
            let ours = get_amount_out(
                &balances,
                &precisions,
                ps,
                d,
                a,
                gamma,
                mid_fee,
                out_fee,
                fg,
                i,
                j,
                amount,
            )
            .expect("ours");
            println!("  {label}: on_chain={on_chain}, ours={ours}");
            assert_eq!(ours, on_chain, "{label} mismatch");
        }

        println!("TwoCryptoNG (crvUSD/FXN) verification passed!");
    }
}
