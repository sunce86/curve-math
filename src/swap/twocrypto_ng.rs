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

pub fn get_amount_in(
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
    desired_output: U256,
) -> Option<U256> {
    if desired_output.is_zero() {
        return None;
    }

    let wad = U256::from(WAD);
    let fee_denom = U256::from(FEE_DENOMINATOR);
    let price_scale_local = price_scale * precisions[1];

    let xp_orig: [U256; 2] = [
        balances[0] * precisions[0],
        balances[1] * price_scale_local / wad,
    ];

    // First pass: estimate fee from pre-swap state
    let fee_est = crypto_fee(&xp_orig, mid_fee, out_fee, fee_gamma)?;
    let complement_est = fee_denom - fee_est;
    let dy_native = (desired_output * fee_denom + complement_est - U256::from(1)) / complement_est;

    let dy_internal = if j > 0 {
        (dy_native * price_scale_local + wad - U256::from(1)) / wad
    } else {
        dy_native * precisions[0]
    } + U256::from(1);
    if xp_orig[j] <= dy_internal {
        return None;
    }
    let y = xp_orig[j] - dy_internal;
    let mut xp_mod = xp_orig;
    xp_mod[j] = y;
    let (x_new, _) = get_y_2_ng(ann, gamma, xp_mod, d, i)?;

    // Second pass: recompute fee with actual xp_after
    let mut xp_after = [U256::ZERO; 2];
    xp_after[i] = x_new;
    xp_after[j] = y;
    let fee_actual = crypto_fee(&xp_after, mid_fee, out_fee, fee_gamma)?;
    let complement_actual = fee_denom - fee_actual;
    let dy_native =
        (desired_output * fee_denom + complement_actual - U256::from(1)) / complement_actual;
    let dy_internal = if j > 0 {
        (dy_native * price_scale_local + wad - U256::from(1)) / wad
    } else {
        dy_native * precisions[0]
    } + U256::from(1);
    if xp_orig[j] <= dy_internal {
        return None;
    }
    let y = xp_orig[j] - dy_internal;
    let mut xp_mod = xp_orig;
    xp_mod[j] = y;
    let (x_new, _) = get_y_2_ng(ann, gamma, xp_mod, d, i)?;

    if x_new <= xp_orig[i] {
        return None;
    }
    let mut dx = if i > 0 {
        (x_new - xp_orig[i]) * wad / price_scale_local
    } else {
        (x_new - xp_orig[i]) / precisions[0]
    } + U256::from(1);
    loop {
        let check = get_amount_out(
            balances,
            precisions,
            price_scale,
            d,
            ann,
            gamma,
            mid_fee,
            out_fee,
            fee_gamma,
            i,
            j,
            dx,
        );
        match check {
            Some(dy_check) if dy_check >= desired_output => break,
            _ => dx += U256::from(1),
        }
    }
    Some(dx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let wad = U256::from(1_000_000_000_000_000_000u128);
        let balances = [U256::from(5000u64) * wad, U256::from(5000u64) * wad];
        let precisions = [U256::from(1u64), U256::from(1u64)];
        let price_scale = wad;
        let d = U256::from(10000u64) * wad;
        let ann = U256::from(540_000u64) * U256::from(10_000u64);
        let gamma = U256::from(11_809_167_828_997u64);
        let mid_fee = U256::from(3_000_000u64);
        let out_fee = U256::from(30_000_000u64);
        let fee_gamma = U256::from(230_000_000_000_000u64);
        let dx = U256::from(1u64) * wad;
        let dy = get_amount_out(
            &balances,
            &precisions,
            price_scale,
            d,
            ann,
            gamma,
            mid_fee,
            out_fee,
            fee_gamma,
            0,
            1,
            dx,
        )
        .expect("out");
        let dx_recovered = get_amount_in(
            &balances,
            &precisions,
            price_scale,
            d,
            ann,
            gamma,
            mid_fee,
            out_fee,
            fee_gamma,
            0,
            1,
            dy,
        )
        .expect("in");
        let dy_check = get_amount_out(
            &balances,
            &precisions,
            price_scale,
            d,
            ann,
            gamma,
            mid_fee,
            out_fee,
            fee_gamma,
            0,
            1,
            dx_recovered,
        )
        .expect("check");
        assert!(dy_check >= dy);
    }

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
