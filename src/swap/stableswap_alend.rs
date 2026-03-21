//! Pool-level get_amount_out for StableSwapALend (Aave, sAAVE, IB, aETH).
//!
//! NO -1. Denorm FIRST, then dynamic fee with avg xp.
//! Uses PRECISION_MUL (not stored_rates/PRECISION).

use alloy_primitives::U256;

use crate::core::stableswap_alend::{dynamic_fee, get_d, get_y, FEE_DENOMINATOR};

pub fn get_amount_out(
    balances: &[U256],
    precision_mul: &[U256],
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
    // Vyper: xp = _balances(); for k: xp[k] *= precisions[k]
    let xp: Vec<U256> = balances
        .iter()
        .zip(precision_mul.iter())
        .map(|(b, p)| *b * *p)
        .collect();
    let d = get_d(&xp, amp)?;
    // Vyper: x = xp[i] + _dx * precisions[i]
    let x_new = xp[i] + dx * precision_mul[i];
    let y_new = get_y(i, j, x_new, &xp, d, amp)?;
    if xp[j] <= y_new {
        return None;
    }
    // NO -1. Denorm FIRST: dy = (xp[j] - y) / precisions[j]
    let dy = (xp[j] - y_new) / precision_mul[j];
    // Dynamic fee with avg xp
    let fee_rate = dynamic_fee(
        (xp[i] + x_new) / U256::from(2),
        (xp[j] + y_new) / U256::from(2),
        fee,
        offpeg_fee_multiplier,
    );
    let fee_amount = fee_rate * dy / U256::from(FEE_DENOMINATOR);
    let result = dy - fee_amount;
    if result.is_zero() {
        return None;
    }
    Some(result)
}

pub fn get_amount_in(
    balances: &[U256],
    precision_mul: &[U256],
    amp: U256,
    fee: U256,
    offpeg_fee_multiplier: U256,
    i: usize,
    j: usize,
    desired_output: U256,
) -> Option<U256> {
    if desired_output.is_zero() {
        return None;
    }
    let fee_denom = U256::from(FEE_DENOMINATOR);
    let xp: Vec<U256> = balances
        .iter()
        .zip(precision_mul.iter())
        .map(|(b, p)| *b * *p)
        .collect();
    let d = get_d(&xp, amp)?;

    // First pass: use base fee as estimate (round up)
    let fee_complement = fee_denom - fee;
    let dy = (desired_output * fee_denom + fee_complement - U256::from(1)) / fee_complement;
    let dy_internal = dy * precision_mul[j];
    if xp[j] <= dy_internal {
        return None;
    }
    let y_new = xp[j] - dy_internal;
    let x_new = get_y(j, i, y_new, &xp, d, amp)?;

    // Second pass: recompute with actual dynamic fee
    let actual_fee = dynamic_fee(
        (xp[i] + x_new) / U256::from(2),
        (xp[j] + y_new) / U256::from(2),
        fee,
        offpeg_fee_multiplier,
    );
    let actual_complement = fee_denom - actual_fee;
    let dy = (desired_output * fee_denom + actual_complement - U256::from(1)) / actual_complement;
    let dy_internal = dy * precision_mul[j];
    if xp[j] <= dy_internal {
        return None;
    }
    let y_new = xp[j] - dy_internal;
    let x_new = get_y(j, i, y_new, &xp, d, amp)?;
    if x_new <= xp[i] {
        return None;
    }
    let dx = (x_new - xp[i]) / precision_mul[i] + U256::from(1);
    Some(dx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::stableswap_alend::A_PRECISION;

    #[test]
    fn roundtrip() {
        // All 18-dec tokens (precision_mul = 1)
        let balances = [
            U256::from(10_000_000_000_000_000_000_000_000u128),
            U256::from(10_000_000_000_000_000_000_000_000u128),
            U256::from(10_000_000_000_000_000_000_000_000u128),
        ];
        let prec_mul = [U256::from(1u64), U256::from(1u64), U256::from(1u64)];
        let amp = U256::from(100u64 * A_PRECISION as u64);
        let fee = U256::from(4_000_000u64);
        let offpeg = U256::from(20_000_000_000u64);
        let dx = U256::from(1_000_000_000_000_000_000_000u128);
        let dy = get_amount_out(&balances, &prec_mul, amp, fee, offpeg, 0, 1, dx).expect("out");
        let dx_recovered =
            get_amount_in(&balances, &prec_mul, amp, fee, offpeg, 0, 1, dy).expect("in");
        assert!(dx_recovered >= dx);
        assert!(dx_recovered <= dx + U256::from(2));
        let dy_check = get_amount_out(&balances, &prec_mul, amp, fee, offpeg, 0, 1, dx_recovered)
            .expect("check");
        assert!(dy_check >= dy);
    }

    alloy::sol! {
        #[sol(rpc)]
        interface ICurvePool {
            function get_dy(int128 i, int128 j, uint256 dx) external view returns (uint256);
            function balances(uint256 i) external view returns (uint256);
            function A() external view returns (uint256);
            function fee() external view returns (uint256);
            function offpeg_fee_multiplier() external view returns (uint256);
        }
    }

    #[tokio::test]
    #[ignore = "requires RPC_URL env var pointing to Ethereum mainnet"]
    async fn verify_aave() {
        use alloy::providers::{Provider, ProviderBuilder};
        use std::str::FromStr;
        let rpc_url = std::env::var("RPC_URL").expect("RPC_URL");
        let provider = ProviderBuilder::new().connect_http(rpc_url.parse().expect("url"));
        let addr =
            alloy_primitives::Address::from_str("0xDeBF20617708857ebe4F679508E7b7863a8A8EeE")
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
        let offpeg = curve
            .offpeg_fee_multiplier()
            .block(block)
            .call()
            .await
            .expect("offpeg");

        let prec_mul = [
            U256::from(1u64),
            U256::from(1_000_000_000_000u64),
            U256::from(1_000_000_000_000u64),
        ];
        let amp = raw_a * U256::from(A_PRECISION as u64);

        let dx = U256::from(1_000_000_000_000_000_000u128);
        let on_chain = curve
            .get_dy(0i128, 1i128, dx)
            .block(block)
            .call()
            .await
            .expect("dy");
        let ours = get_amount_out(&[r0, r1, r2], &prec_mul, amp, pool_fee, offpeg, 0, 1, dx)
            .expect("ours");
        println!("  aDAI→aUSDC: on_chain={on_chain}, ours={ours}");
        assert_eq!(ours, on_chain, "mismatch");
        println!("StableSwapALend (Aave) passed!");
    }
}
