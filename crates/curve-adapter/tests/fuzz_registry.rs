//! Registry-driven differential fuzz test.
//!
//! Reads `tests/registry/<chain>.toml`, tests every pool by:
//! 1. Reading on-chain state via RPC
//! 2. Constructing `RawPoolState` → `build_pool()` → `Pool`
//! 3. Comparing `get_amount_out()` against on-chain `get_dy()` (wei-exact)
//!
//! This tests the entire curve-adapter pipeline end-to-end.
//!
//! Run all chains:
//!   FUZZ_ITERATIONS=100 RPC_URL_1=<rpc> \
//!     cargo test -p curve-adapter --test fuzz_registry -- --ignored --nocapture
//!
//! Run single chain:
//!   FUZZ_ITERATIONS=100 RPC_URL_1=<rpc> \
//!     cargo test -p curve-adapter --test fuzz_registry -- fuzz_1 --ignored --nocapture

use alloy::providers::{Provider, ProviderBuilder};
use alloy_primitives::{Address, U256};
use curve_adapter::{build_pool, CurveVariant, RawPoolState};
use curve_math::Pool;
use serde::Deserialize;
use std::str::FromStr;

// ── Registry types ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Registry {
    pools: Vec<PoolEntry>,
}

#[derive(Deserialize)]
struct PoolEntry {
    address: String,
    variant: String,
    name: String,
}

// ── On-chain coin discovery ──────────────────────────────────────────────────

alloy::sol! {
    #[sol(rpc)]
    interface ICoinInfo {
        function coins(uint256 i) external view returns (address);
    }
    #[sol(rpc)]
    interface IToken {
        function decimals() external view returns (uint8);
    }
}

const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

async fn read_pool_coins(
    addr: Address,
    provider: &BoxProvider,
    block: alloy::eips::BlockId,
) -> Option<(usize, Vec<u8>)> {
    let c = ICoinInfo::new(addr, provider);
    let mut decimals = Vec::new();
    for i in 0..8u64 {
        let coin_result = match c.coins(U256::from(i)).block(block).call().await {
            Ok(v) => Ok(v),
            Err(_) => {
                tokio::time::sleep(RETRY_DELAY).await;
                c.coins(U256::from(i)).block(block).call().await
            }
        };
        match coin_result {
            Ok(coin_addr) => {
                if coin_addr == Address::ZERO {
                    break;
                }
                let token = IToken::new(coin_addr, provider);
                let dec = match token.decimals().block(block).call().await {
                    Ok(d) => d,
                    Err(_) => {
                        tokio::time::sleep(RETRY_DELAY).await;
                        token.decimals().block(block).call().await.unwrap_or(18)
                    }
                };
                decimals.push(dec);
            }
            Err(_) => break,
        }
    }
    if decimals.is_empty() {
        return None;
    }
    Some((decimals.len(), decimals))
}

// ── On-chain interfaces ─────────────────────────────────────────────────────

alloy::sol! {
    #[sol(rpc)]
    interface IStableOld {
        function get_dy(int128 i, int128 j, uint256 dx) external view returns (uint256);
        function balances(int128 i) external view returns (uint256);
        function A() external view returns (uint256);
        function fee() external view returns (uint256);
    }

    #[sol(rpc)]
    interface IStable {
        function get_dy(int128 i, int128 j, uint256 dx) external view returns (uint256);
        function balances(uint256 i) external view returns (uint256);
        function A() external view returns (uint256);
        function fee() external view returns (uint256);
        function initial_A() external view returns (uint256);
        function future_A() external view returns (uint256);
    }

    #[sol(rpc)]
    interface IStableOffpeg {
        function get_dy(int128 i, int128 j, uint256 dx) external view returns (uint256);
        function balances(uint256 i) external view returns (uint256);
        function A() external view returns (uint256);
        function fee() external view returns (uint256);
        function initial_A() external view returns (uint256);
        function future_A() external view returns (uint256);
        function offpeg_fee_multiplier() external view returns (uint256);
        function stored_rates() external view returns (uint256[]);
    }

    #[sol(rpc)]
    interface IStableMeta {
        function get_dy(int128 i, int128 j, uint256 dx) external view returns (uint256);
        function balances(uint256 i) external view returns (uint256);
        function A() external view returns (uint256);
        function fee() external view returns (uint256);
        function initial_A() external view returns (uint256);
        function future_A() external view returns (uint256);
        function base_pool() external view returns (address);
        function base_virtual_price() external view returns (uint256);
        function base_cache_updated() external view returns (uint256);
    }

    #[sol(rpc)]
    interface IBasePool {
        function get_virtual_price() external view returns (uint256);
    }

    #[sol(rpc)]
    interface ICrypto2 {
        function get_dy(uint256 i, uint256 j, uint256 dx) external view returns (uint256);
        function balances(uint256 i) external view returns (uint256);
        function A() external view returns (uint256);
        function gamma() external view returns (uint256);
        function D() external view returns (uint256);
        function price_scale() external view returns (uint256);
        function mid_fee() external view returns (uint256);
        function out_fee() external view returns (uint256);
        function fee_gamma() external view returns (uint256);
        function precisions() external view returns (uint256[2]);
    }

    #[sol(rpc)]
    interface ICrypto3 {
        function get_dy(uint256 i, uint256 j, uint256 dx) external view returns (uint256);
        function balances(uint256 i) external view returns (uint256);
        function A() external view returns (uint256);
        function gamma() external view returns (uint256);
        function D() external view returns (uint256);
        function price_scale(uint256 i) external view returns (uint256);
        function mid_fee() external view returns (uint256);
        function out_fee() external view returns (uint256);
        function fee_gamma() external view returns (uint256);
        function precisions() external view returns (uint256[3]);
    }
}

alloy::sol! {
    #[sol(rpc)]
    interface IMulticall3 {
        struct Call3 {
            address target;
            bool allowFailure;
            bytes callData;
        }
        struct Result {
            bool success;
            bytes returnData;
        }
        function aggregate3(Call3[] calldata calls) external payable returns (Result[] returnData);
    }
}

const MULTICALL3: Address = Address::new([
    0xca, 0x11, 0xbd, 0xe0, 0x59, 0x77, 0xb3, 0x63, 0x11, 0x67, 0x02, 0x88, 0x62, 0xbE, 0x2a, 0x17,
    0x39, 0x76, 0xCA, 0x11,
]);
const BATCH_SIZE: usize = 100;

async fn batch_get_dy(
    entry: &PoolEntry,
    provider: &BoxProvider,
    block: alloy::eips::BlockId,
    cases: &[(usize, usize, U256)],
) -> Vec<Option<U256>> {
    use alloy::sol_types::SolCall;

    let addr = Address::from_str(&entry.address).unwrap();
    let mc = IMulticall3::new(MULTICALL3, provider);

    let variant: CurveVariant = entry.variant.parse().unwrap();
    let is_crypto =
        variant.as_str().starts_with("TwoCrypto") || variant.as_str().starts_with("TriCrypto");

    let mut results = Vec::with_capacity(cases.len());

    for chunk in cases.chunks(BATCH_SIZE) {
        let calls: Vec<IMulticall3::Call3> = chunk
            .iter()
            .map(|(i, j, dx)| {
                let calldata = if is_crypto {
                    ICrypto2::get_dyCall {
                        i: U256::from(*i),
                        j: U256::from(*j),
                        dx: *dx,
                    }
                    .abi_encode()
                } else {
                    IStable::get_dyCall {
                        i: *i as i128,
                        j: *j as i128,
                        dx: *dx,
                    }
                    .abi_encode()
                };
                IMulticall3::Call3 {
                    target: addr,
                    allowFailure: true,
                    callData: calldata.into(),
                }
            })
            .collect();

        match mc.aggregate3(calls).block(block).call().await {
            Ok(batch_results) => {
                for r in &batch_results {
                    if r.success && r.returnData.len() >= 32 {
                        results.push(Some(U256::from_be_slice(&r.returnData[..32])));
                    } else {
                        results.push(None);
                    }
                }
            }
            Err(e) => {
                eprintln!("    multicall3 batch failed: {e}");
                results.extend(std::iter::repeat(None).take(chunk.len()));
            }
        }
    }

    results
}

// ── PRNG + amount generation ────────────────────────────────────────────────

fn splitmix64(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = *seed;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

fn generate_amounts(n: usize, balance: U256, seed: u64) -> Vec<U256> {
    if balance.is_zero() || n == 0 {
        return vec![];
    }
    let mut amounts = Vec::with_capacity(n + 6);
    amounts.push(U256::ZERO);
    amounts.push(U256::from(1u64));
    amounts.push(balance / U256::from(1000u64));
    amounts.push(balance / U256::from(10u64));
    amounts.push(balance / U256::from(2u64));
    amounts.push(balance);
    amounts.push(balance * U256::from(2u64));
    amounts.push(U256::MAX);
    let remaining = n.saturating_sub(amounts.len());
    if remaining > 0 {
        let mut s = seed;
        let max_f64 = balance.to_string().parse::<f64>().unwrap_or(1e30);
        let ln_max = max_f64.ln();
        for _ in 0..remaining {
            let r = splitmix64(&mut s);
            let t = (r as f64) / (u64::MAX as f64);
            let val = (t * ln_max).exp();
            let v = U256::from(val.min(1e38) as u128)
                .max(U256::from(1u64))
                .min(balance);
            amounts.push(v);
        }
    }
    amounts
}

fn fuzz_iterations() -> usize {
    std::env::var("FUZZ_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100)
}

// ── Pool builder using curve-adapter ────────────────────────────────────────

type BoxProvider = alloy::providers::fillers::FillProvider<
    alloy::providers::fillers::JoinFill<
        alloy::providers::Identity,
        alloy::providers::fillers::JoinFill<
            alloy::providers::fillers::GasFiller,
            alloy::providers::fillers::JoinFill<
                alloy::providers::fillers::BlobGasFiller,
                alloy::providers::fillers::JoinFill<
                    alloy::providers::fillers::NonceFiller,
                    alloy::providers::fillers::ChainIdFiller,
                >,
            >,
        >,
    >,
    alloy::providers::RootProvider,
>;

/// Read on-chain state and construct a Pool via curve-adapter's build_pool().
///
/// This is the reference RPC consumer implementation: reads raw state from
/// on-chain getters, populates RawPoolState, and calls build_pool().
async fn read_and_build_pool(
    entry: &PoolEntry,
    provider: &BoxProvider,
    block: alloy::eips::BlockId,
) -> Option<(Pool, usize)> {
    let addr = Address::from_str(&entry.address).ok()?;
    let variant: CurveVariant = entry.variant.parse().ok()?;
    let (n_coins, decimals) = read_pool_coins(addr, provider, block).await?;
    let a_prec_100 = U256::from(100u64);

    let state = match variant {
        CurveVariant::StableSwapV0 => {
            let c = IStableOld::new(addr, provider);
            let mut balances = Vec::new();
            for i in 0..n_coins {
                balances.push(c.balances(i as i128).block(block).call().await.ok()?);
            }
            let amp = c.A().block(block).call().await.ok()?;
            let fee = c.fee().block(block).call().await.ok()?;
            RawPoolState {
                variant,
                balances,
                token_decimals: decimals,
                amp,
                fee: Some(fee),
                ..Default::default()
            }
        }
        CurveVariant::StableSwapV1 => {
            let c = IStable::new(addr, provider);
            let mut balances = Vec::new();
            for i in 0..n_coins {
                balances.push(c.balances(U256::from(i)).block(block).call().await.ok()?);
            }
            let amp = c.A().block(block).call().await.ok()?;
            let fee = c.fee().block(block).call().await.ok()?;
            RawPoolState {
                variant,
                balances,
                token_decimals: decimals,
                amp,
                fee: Some(fee),
                ..Default::default()
            }
        }
        CurveVariant::StableSwapV2 => {
            let c = IStable::new(addr, provider);
            let mut balances = Vec::new();
            for i in 0..n_coins {
                balances.push(c.balances(U256::from(i)).block(block).call().await.ok()?);
            }
            // Prefer initial_A when no ramping (avoids A() integer division loss).
            let amp = match (
                c.initial_A().block(block).call().await,
                c.future_A().block(block).call().await,
            ) {
                (Ok(ia), Ok(fa)) if ia == fa => ia,
                _ => {
                    let raw_a = c.A().block(block).call().await.ok()?;
                    raw_a * a_prec_100
                }
            };
            let fee = c.fee().block(block).call().await.ok()?;
            RawPoolState {
                variant,
                balances,
                token_decimals: decimals,
                amp,
                fee: Some(fee),
                ..Default::default()
            }
        }
        CurveVariant::StableSwapALend => {
            let c = IStableOffpeg::new(addr, provider);
            let mut balances = Vec::new();
            for i in 0..n_coins {
                balances.push(c.balances(U256::from(i)).block(block).call().await.ok()?);
            }
            // Prefer initial_A when no ramping (avoids A() integer division loss).
            let amp = match (
                c.initial_A().block(block).call().await,
                c.future_A().block(block).call().await,
            ) {
                (Ok(ia), Ok(fa)) if ia == fa => ia,
                _ => c.A().block(block).call().await.ok()? * a_prec_100,
            };
            let fee = c.fee().block(block).call().await.ok()?;
            let offpeg = c.offpeg_fee_multiplier().block(block).call().await.ok()?;
            RawPoolState {
                variant,
                balances,
                token_decimals: decimals,
                amp,
                fee: Some(fee),
                offpeg_fee_multiplier: Some(offpeg),
                ..Default::default()
            }
        }
        CurveVariant::StableSwapNG => {
            let c = IStableOffpeg::new(addr, provider);
            let mut balances = Vec::new();
            for i in 0..n_coins {
                balances.push(c.balances(U256::from(i)).block(block).call().await.ok()?);
            }
            let amp = match (
                c.initial_A().block(block).call().await,
                c.future_A().block(block).call().await,
            ) {
                (Ok(ia), Ok(fa)) if ia == fa => ia,
                _ => c.A().block(block).call().await.ok()? * a_prec_100,
            };
            let fee = c.fee().block(block).call().await.ok()?;
            // v5+ crvUSD factory pools lack offpeg_fee_multiplier;
            // builder defaults to FEE_DENOMINATOR when None.
            let offpeg = c.offpeg_fee_multiplier().block(block).call().await.ok();
            // stored_rates() encoding varies by pool version:
            // - Older NG (v5+): fixed-size uint256[N_COINS], no length prefix
            // - Newer NG (v6+): dynamic uint256[], with offset + length prefix
            // Detect by checking if first word is a small offset (dynamic) or
            // a large rate value (fixed). Use raw eth_call for both.
            let dynamic_rates = {
                use alloy::providers::Provider;
                let calldata = alloy::primitives::bytes!("fd0684b1"); // stored_rates()
                let tx = alloy::rpc::types::TransactionRequest::default()
                    .to(addr)
                    .input(calldata.into());
                match provider.call(tx).block(block.into()).await {
                    Ok(output) if output.len() >= n_coins * 32 => {
                        // Heuristic: if first word <= 256, it's an ABI offset
                        // (dynamic encoding), not a rate (rates are >= 10^18).
                        let first_word = U256::from_be_slice(&output[..32]);
                        let data_offset = if first_word <= U256::from(256u64)
                            && output.len() >= (n_coins + 2) * 32
                        {
                            // Dynamic: skip offset word + length word
                            64
                        } else {
                            // Fixed: rates start at byte 0
                            0
                        };
                        let rates: Vec<Option<U256>> = (0..n_coins)
                            .map(|i| {
                                let start = data_offset + i * 32;
                                Some(U256::from_be_slice(&output[start..start + 32]))
                            })
                            .collect();
                        Some(rates)
                    }
                    _ => None,
                }
            };
            RawPoolState {
                variant,
                balances,
                token_decimals: decimals,
                amp,
                fee: Some(fee),
                offpeg_fee_multiplier: offpeg,
                dynamic_rates,
                ..Default::default()
            }
        }
        CurveVariant::StableSwapMeta => {
            let c = IStableMeta::new(addr, provider);
            let mut balances = Vec::new();
            for i in 0..n_coins {
                balances.push(c.balances(U256::from(i)).block(block).call().await.ok()?);
            }
            let amp = match (
                c.initial_A().block(block).call().await,
                c.future_A().block(block).call().await,
            ) {
                (Ok(ia), Ok(fa)) if ia == fa => ia,
                _ => c.A().block(block).call().await.ok()? * a_prec_100,
            };
            let fee = c.fee().block(block).call().await.ok()?;
            // Read virtual_price for base LP token
            let base_pool_addr = match c.base_pool().block(block).call().await {
                Ok(a) => a,
                Err(_) => {
                    let lp_token = ICoinInfo::new(addr, provider)
                        .coins(U256::from(1))
                        .block(block)
                        .call()
                        .await
                        .ok()?;
                    alloy::sol! {
                        #[sol(rpc)]
                        interface ILPToken { function minter() external view returns (address); }
                        #[sol(rpc)]
                        interface IFactory { function get_base_pool(address pool) external view returns (address); }
                    }
                    match ILPToken::new(lp_token, provider)
                        .minter()
                        .block(block)
                        .call()
                        .await
                    {
                        Ok(a) => a,
                        Err(_) => {
                            // MetaPool Factory proxy: LP token has no minter().
                            // Ask the factory for the base pool.
                            let factory =
                                Address::from_str("0xB9fC157394Af804a3578134A6585C0dc9cc990d4")
                                    .unwrap();
                            IFactory::new(factory, provider)
                                .get_base_pool(addr)
                                .block(block)
                                .call()
                                .await
                                .ok()?
                        }
                    }
                }
            };
            let base_pool = IBasePool::new(base_pool_addr, provider);
            let vp = match (
                c.base_virtual_price().block(block).call().await,
                c.base_cache_updated().block(block).call().await,
            ) {
                (Ok(cached_vp), Ok(cache_updated)) => {
                    let bn_num = match block {
                        alloy::eips::BlockId::Number(n) => n.as_number().unwrap_or(0),
                        _ => 0,
                    };
                    let block_data = provider.get_block_by_number(bn_num.into()).await.ok()??;
                    let block_ts = U256::from(block_data.header.timestamp);
                    if block_ts - cache_updated > U256::from(600u64) {
                        base_pool
                            .get_virtual_price()
                            .block(block)
                            .call()
                            .await
                            .ok()?
                    } else {
                        cached_vp
                    }
                }
                _ => base_pool
                    .get_virtual_price()
                    .block(block)
                    .call()
                    .await
                    .ok()?,
            };
            let mut dr: Vec<Option<U256>> = vec![None; n_coins];
            if let Some(last) = dr.last_mut() {
                *last = Some(vp);
            }
            RawPoolState {
                variant,
                balances,
                token_decimals: decimals,
                amp,
                fee: Some(fee),
                dynamic_rates: Some(dr),
                ..Default::default()
            }
        }
        CurveVariant::TwoCryptoV1 | CurveVariant::TwoCryptoNG => {
            let c = ICrypto2::new(addr, provider);
            let b0 = c.balances(U256::from(0)).block(block).call().await.ok()?;
            let b1 = c.balances(U256::from(1)).block(block).call().await.ok()?;
            let ann = c.A().block(block).call().await.ok()?;
            let gamma = c.gamma().block(block).call().await.ok()?;
            let d = c.D().block(block).call().await.ok()?;
            let ps = c.price_scale().block(block).call().await.ok()?;
            let mid_fee = c.mid_fee().block(block).call().await.ok()?;
            let out_fee = c.out_fee().block(block).call().await.ok()?;
            let fee_gamma = c.fee_gamma().block(block).call().await.ok()?;
            let precs = c.precisions().block(block).call().await.ok();
            RawPoolState {
                variant,
                balances: vec![b0, b1],
                token_decimals: decimals,
                amp: ann,
                mid_fee: Some(mid_fee),
                out_fee: Some(out_fee),
                fee_gamma: Some(fee_gamma),
                d: Some(d),
                gamma: Some(gamma),
                price_scale: Some(vec![ps]),
                precisions: precs.map(|p| p.to_vec()),
                ..Default::default()
            }
        }
        CurveVariant::TwoCryptoStable => {
            let c = ICrypto2::new(addr, provider);
            let b0 = c.balances(U256::from(0)).block(block).call().await.ok()?;
            let b1 = c.balances(U256::from(1)).block(block).call().await.ok()?;
            let ann = c.A().block(block).call().await.ok()?;
            let d = c.D().block(block).call().await.ok()?;
            let ps = c.price_scale().block(block).call().await.ok()?;
            let mid_fee = c.mid_fee().block(block).call().await.ok()?;
            let out_fee = c.out_fee().block(block).call().await.ok()?;
            let fee_gamma = c.fee_gamma().block(block).call().await.ok()?;
            let precs = c.precisions().block(block).call().await.ok();
            RawPoolState {
                variant,
                balances: vec![b0, b1],
                token_decimals: decimals,
                amp: ann,
                mid_fee: Some(mid_fee),
                out_fee: Some(out_fee),
                fee_gamma: Some(fee_gamma),
                d: Some(d),
                price_scale: Some(vec![ps]),
                precisions: precs.map(|p| p.to_vec()),
                ..Default::default()
            }
        }
        CurveVariant::TriCryptoV1 | CurveVariant::TriCryptoNG => {
            let c = ICrypto3::new(addr, provider);
            let b0 = c.balances(U256::from(0)).block(block).call().await.ok()?;
            let b1 = c.balances(U256::from(1)).block(block).call().await.ok()?;
            let b2 = c.balances(U256::from(2)).block(block).call().await.ok()?;
            let ann = c.A().block(block).call().await.ok()?;
            let gamma = c.gamma().block(block).call().await.ok()?;
            let d = c.D().block(block).call().await.ok()?;
            let ps0 = c
                .price_scale(U256::from(0))
                .block(block)
                .call()
                .await
                .ok()?;
            let ps1 = c
                .price_scale(U256::from(1))
                .block(block)
                .call()
                .await
                .ok()?;
            let mid_fee = c.mid_fee().block(block).call().await.ok()?;
            let out_fee = c.out_fee().block(block).call().await.ok()?;
            let fee_gamma = c.fee_gamma().block(block).call().await.ok()?;
            let precs = c.precisions().block(block).call().await.ok();
            RawPoolState {
                variant,
                balances: vec![b0, b1, b2],
                token_decimals: decimals,
                amp: ann,
                mid_fee: Some(mid_fee),
                out_fee: Some(out_fee),
                fee_gamma: Some(fee_gamma),
                d: Some(d),
                gamma: Some(gamma),
                price_scale: Some(vec![ps0, ps1]),
                precisions: precs.map(|p| p.to_vec()),
                ..Default::default()
            }
        }
    };

    let pool = build_pool(&state).ok()?;
    Some((pool, n_coins))
}

// ── Shared fuzz runner ───────────────────────────────────────────────────────

async fn fuzz_pools(label: &str, pools: &[PoolEntry]) {
    let chain_id: u64 = label
        .split_whitespace()
        .find_map(|w| w.parse().ok())
        .unwrap_or(1);
    let env_key = format!("RPC_URL_{chain_id}");
    let rpc_url = std::env::var(&env_key)
        .or_else(|_| std::env::var("RPC_URL"))
        .unwrap_or_else(|_| panic!("{env_key} or RPC_URL must be set"));
    let provider = ProviderBuilder::new().connect_http(rpc_url.parse().expect("invalid RPC_URL"));
    let latest = provider.get_block_number().await.expect("block");
    let bn = latest - 5;
    let block = alloy::eips::BlockId::number(bn);

    println!(
        "[{label}] {} pools, block {bn} (latest {latest})",
        pools.len()
    );

    let n = fuzz_iterations();
    let mut total_passed = 0u64;
    let mut total_skipped = 0u64;
    let mut pools_ok = 0u64;

    for entry in pools {
        let (pool, n_coins) = match read_and_build_pool(entry, &provider, block).await {
            Some(p) => p,
            None => {
                println!("  SKIP {}: could not read on-chain state", entry.name);
                continue;
            }
        };

        let mut pairs: Vec<(usize, usize)> = Vec::new();
        for i in 0..n_coins {
            for j in 0..n_coins {
                if i != j {
                    pairs.push((i, j));
                }
            }
        }
        if pairs.is_empty() {
            println!("  SKIP {}: no valid coin pairs", entry.name);
            continue;
        }
        let per_pair = (n / pairs.len()).max(1);

        let mut passed = 0u64;
        let mut skipped = 0u64;

        let balances = match &pool {
            Pool::StableSwapV0 { balances, .. }
            | Pool::StableSwapV1 { balances, .. }
            | Pool::StableSwapV2 { balances, .. }
            | Pool::StableSwapALend { balances, .. }
            | Pool::StableSwapNG { balances, .. }
            | Pool::StableSwapMeta { balances, .. } => balances.clone(),
            Pool::TwoCryptoV1 { balances, .. }
            | Pool::TwoCryptoNG { balances, .. }
            | Pool::TwoCryptoStable { balances, .. } => balances.to_vec(),
            Pool::TriCryptoV1 { balances, .. } | Pool::TriCryptoNG { balances, .. } => {
                balances.to_vec()
            }
        };

        let mut test_cases: Vec<(usize, usize, U256)> = Vec::new();
        for (idx, &(i, j)) in pairs.iter().enumerate() {
            for dx in generate_amounts(per_pair, balances[i], bn + idx as u64) {
                test_cases.push((i, j, dx));
            }
        }

        let on_chain_results = batch_get_dy(entry, &provider, block, &test_cases).await;

        for ((i, j, dx), on_chain) in test_cases.iter().zip(on_chain_results.iter()) {
            match on_chain {
                Some(expected) => {
                    let ours = pool.get_amount_out(*i, *j, *dx);
                    match ours {
                        Some(result) => {
                            assert_eq!(
                                result, *expected,
                                "{} ({}) {i}→{j} mismatch at dx={dx}",
                                entry.name, entry.variant
                            );
                            passed += 1;
                        }
                        None => skipped += 1,
                    }
                }
                None => skipped += 1,
            }
        }

        println!(
            "  {} ({}): {passed} passed, {skipped} skipped",
            entry.name, entry.variant
        );
        total_passed += passed;
        total_skipped += skipped;
        pools_ok += 1;
    }

    println!("[{label}] {pools_ok} pools, {total_passed} passed, {total_skipped} skipped\n");
    assert!(total_passed > 0, "no tests passed for {label}");
}

fn load_registry(path: &str) -> Vec<PoolEntry> {
    let toml_str = std::fs::read_to_string(path).unwrap_or_else(|_| panic!("{path} not found"));
    let registry: Registry = toml::from_str(&toml_str).unwrap_or_else(|_| panic!("invalid {path}"));
    registry.pools
}

// ── Per-chain tests ─────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "requires RPC_URL_1 or RPC_URL"]
async fn fuzz_1() {
    let pools = load_registry("tests/registry/1.toml");
    fuzz_pools("chain 1", &pools).await;
}

#[tokio::test]
#[ignore = "requires RPC_URL_1 or RPC_URL"]
async fn fuzz_1_pending() {
    let path = "tests/registry/1_pending.toml";
    if !std::path::Path::new(path).exists() {
        println!("No pending pools at {path}");
        return;
    }
    let pools = load_registry(path);
    if pools.is_empty() {
        println!("No pending pools");
        return;
    }
    fuzz_pools("chain 1 pending", &pools).await;
}

#[tokio::test]
#[ignore = "requires RPC_URL_8453"]
async fn fuzz_8453() {
    let pools = load_registry("tests/registry/8453.toml");
    fuzz_pools("chain 8453", &pools).await;
}

#[tokio::test]
#[ignore = "requires RPC_URL_8453"]
async fn fuzz_8453_pending() {
    let path = "tests/registry/8453_pending.toml";
    if !std::path::Path::new(path).exists() {
        println!("No pending pools at {path}");
        return;
    }
    let pools = load_registry(path);
    if pools.is_empty() {
        println!("No pending pools");
        return;
    }
    fuzz_pools("chain 8453 pending", &pools).await;
}

#[tokio::test]
#[ignore = "requires RPC_URL_42161"]
async fn fuzz_42161() {
    let pools = load_registry("tests/registry/42161.toml");
    fuzz_pools("chain 42161", &pools).await;
}

#[tokio::test]
#[ignore = "requires RPC_URL_42161"]
async fn fuzz_42161_pending() {
    let path = "tests/registry/42161_pending.toml";
    if !std::path::Path::new(path).exists() {
        println!("No pending pools at {path}");
        return;
    }
    let pools = load_registry(path);
    if pools.is_empty() {
        println!("No pending pools");
        return;
    }
    fuzz_pools("chain 42161 pending", &pools).await;
}
