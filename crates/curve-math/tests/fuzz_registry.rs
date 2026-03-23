//! Registry-driven differential fuzz test.
//!
//! Reads `registry/<chain>.toml`, tests every pool with `fuzz_verified = true`.
//! One generic test per chain — no per-variant copy-paste.
//!
//! Run all chains:
//!   FUZZ_ITERATIONS=100 RPC_URL_ETHEREUM=<rpc> \
//!     cargo test --features swap --test fuzz_registry -- --ignored --nocapture
//!
//! Run single chain:
//!   FUZZ_ITERATIONS=100 RPC_URL_ETHEREUM=<rpc> \
//!     cargo test --features swap --test fuzz_registry -- fuzz_ethereum --ignored --nocapture

#![cfg(feature = "swap")]

use alloy::providers::{Provider, ProviderBuilder};
use alloy_primitives::{Address, U256};
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

/// Read coins count and decimals from on-chain pool contract.
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
        function initial_A_time() external view returns (uint256);
        function future_A_time() external view returns (uint256);
    }

    #[sol(rpc)]
    interface IStableOffpeg {
        function get_dy(int128 i, int128 j, uint256 dx) external view returns (uint256);
        function balances(uint256 i) external view returns (uint256);
        function A() external view returns (uint256);
        function fee() external view returns (uint256);
        function offpeg_fee_multiplier() external view returns (uint256);
        function stored_rates() external view returns (uint256[]);
    }

    #[sol(rpc)]
    interface IStableMeta {
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
        function MATH() external view returns (address);
    }

    #[sol(rpc)]
    interface IMathVersion {
        function version() external view returns (string);
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

/// Batch `get_dy` calls via Multicall3 to reduce RPC request count.
/// Returns one `Option<U256>` per input case (None = on-chain revert or batch error).
async fn batch_get_dy(
    entry: &PoolEntry,
    provider: &BoxProvider,
    block: alloy::eips::BlockId,
    cases: &[(usize, usize, U256)],
) -> Vec<Option<U256>> {
    use alloy::sol_types::SolCall;

    let addr = Address::from_str(&entry.address).unwrap();
    let mc = IMulticall3::new(MULTICALL3, provider);

    let is_crypto = matches!(
        entry.variant.as_str(),
        "TwoCryptoV1" | "TwoCryptoNG" | "TwoCryptoStable" | "TriCryptoV1" | "TriCryptoNG"
    );

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
                        // Take first 32 bytes — some older Vyper contracts
                        // return padded data (e.g. 4096 bytes).
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

fn rate_for_decimals(dec: u8) -> U256 {
    // rate = 10^(36 - decimals) / 10^18 = 10^(18 - decimals) ... but stored as 10^(36-dec)/PRECISION
    // For StableSwap: rate = PRECISION * PRECISION_MUL = 10^18 * 10^(18-dec) = 10^(36-dec)
    // Actually: rate = 10^(36 - decimals) for the rates-based variants
    // For 18-dec: 10^18, for 6-dec: 10^30, for 8-dec: 10^28, for 2-dec: 10^34
    U256::from(10u64).pow(U256::from(36 - dec as u32))
}

fn precision_mul_for_decimals(dec: u8) -> U256 {
    U256::from(10u64).pow(U256::from(18 - dec as u32))
}

fn precision_for_decimals(dec: u8) -> U256 {
    // CryptoSwap precisions: 10^(18 - decimals)
    U256::from(10u64).pow(U256::from(18 - dec as u32))
}

// ── Generic pool builder ────────────────────────────────────────────────────

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

async fn build_pool(
    entry: &PoolEntry,
    provider: &BoxProvider,
    block: alloy::eips::BlockId,
) -> Option<(Pool, usize)> {
    let addr = Address::from_str(&entry.address).ok()?;
    let a_prec_100 = U256::from(100u64);
    let (n_coins, decimals) = read_pool_coins(addr, provider, block).await?;

    match entry.variant.as_str() {
        "StableSwapV0" => {
            let c = IStableOld::new(addr, provider);
            let mut balances = Vec::new();
            for i in 0..n_coins {
                balances.push(c.balances(i as i128).block(block).call().await.ok()?);
            }
            let amp = c.A().block(block).call().await.ok()?;
            let fee = c.fee().block(block).call().await.ok()?;
            let rates: Vec<U256> = decimals.iter().map(|d| rate_for_decimals(*d)).collect();
            Some((
                Pool::StableSwapV0 {
                    balances,
                    rates,
                    amp,
                    fee,
                },
                n_coins,
            ))
        }
        "StableSwapV1" => {
            let c = IStable::new(addr, provider);
            let mut balances = Vec::new();
            for i in 0..n_coins {
                balances.push(c.balances(U256::from(i)).block(block).call().await.ok()?);
            }
            let amp = c.A().block(block).call().await.ok()?;
            let fee = c.fee().block(block).call().await.ok()?;
            let rates: Vec<U256> = decimals.iter().map(|d| rate_for_decimals(*d)).collect();
            Some((
                Pool::StableSwapV1 {
                    balances,
                    rates,
                    amp,
                    fee,
                },
                n_coins,
            ))
        }
        "StableSwapV2" => {
            let c = IStable::new(addr, provider);
            let mut balances = Vec::new();
            for i in 0..n_coins {
                balances.push(c.balances(U256::from(i)).block(block).call().await.ok()?);
            }
            // Prefer initial_A when no ramping (avoids A() integer division precision loss).
            // When ramping is active (initial_A != future_A), fall back to A() * A_PRECISION.
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
            let rates: Vec<U256> = decimals.iter().map(|d| rate_for_decimals(*d)).collect();
            Some((
                Pool::StableSwapV2 {
                    balances,
                    rates,
                    amp,
                    fee,
                },
                n_coins,
            ))
        }
        "StableSwapALend" => {
            let c = IStableOffpeg::new(addr, provider);
            let mut balances = Vec::new();
            for i in 0..n_coins {
                balances.push(c.balances(U256::from(i)).block(block).call().await.ok()?);
            }
            let raw_a = c.A().block(block).call().await.ok()?;
            let fee = c.fee().block(block).call().await.ok()?;
            let offpeg = c.offpeg_fee_multiplier().block(block).call().await.ok()?;
            let precision_mul: Vec<U256> = decimals
                .iter()
                .map(|d| precision_mul_for_decimals(*d))
                .collect();
            let amp = raw_a * a_prec_100;
            Some((
                Pool::StableSwapALend {
                    balances,
                    precision_mul,
                    amp,
                    fee,
                    offpeg_fee_multiplier: offpeg,
                },
                n_coins,
            ))
        }
        "StableSwapNG" => {
            let c = IStableOffpeg::new(addr, provider);
            let mut balances = Vec::new();
            for i in 0..n_coins {
                balances.push(c.balances(U256::from(i)).block(block).call().await.ok()?);
            }
            let raw_a = c.A().block(block).call().await.ok()?;
            let fee = c.fee().block(block).call().await.ok()?;
            let offpeg = c.offpeg_fee_multiplier().block(block).call().await.ok()?;
            // Try stored_rates() first (handles ERC4626/oracle tokens)
            // Falls back to static rates from decimals
            let rates: Vec<U256> = match c.stored_rates().block(block).call().await {
                Ok(r) if r.len() == n_coins => r,
                _ => decimals.iter().map(|d| rate_for_decimals(*d)).collect(),
            };
            let amp = raw_a * a_prec_100;
            Some((
                Pool::StableSwapNG {
                    balances,
                    rates,
                    amp,
                    fee,
                    offpeg_fee_multiplier: offpeg,
                },
                n_coins,
            ))
        }
        "StableSwapMeta" => {
            let c = IStableMeta::new(addr, provider);
            let mut balances = Vec::new();
            for i in 0..n_coins {
                balances.push(c.balances(U256::from(i)).block(block).call().await.ok()?);
            }
            let raw_a = c.A().block(block).call().await.ok()?;
            let fee = c.fee().block(block).call().await.ok()?;
            // Read virtual_price for base LP token rate
            // Newer meta pools have base_pool(), older ones don't.
            // Fallback: read coin[1] (LP token) → call minter() → that's the base pool
            let base_pool_addr = match c.base_pool().block(block).call().await {
                Ok(addr) => addr,
                Err(_) => {
                    // Legacy meta pool — find base pool via LP token's minter()
                    let lp_token = ICoinInfo::new(addr, provider)
                        .coins(U256::from(1))
                        .block(block)
                        .call()
                        .await
                        .ok()?;
                    alloy::sol! {
                        #[sol(rpc)]
                        interface ILPToken { function minter() external view returns (address); }
                    }
                    match ILPToken::new(lp_token, provider)
                        .minter()
                        .block(block)
                        .call()
                        .await
                    {
                        Ok(a) => a,
                        Err(_) => {
                            // CurveTokenV2 (3Crv) has no minter() — fallback to 3pool
                            Address::from_str("0xbEbc44782C7dB0a1A60Cb6fe97d0b483032FF1C7").unwrap()
                        }
                    }
                }
            };
            let base_pool = IBasePool::new(base_pool_addr, provider);
            // Try cached vp with staleness check, fallback to direct get_virtual_price
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
                _ => {
                    // Legacy meta pool — read virtual_price directly from base pool
                    base_pool
                        .get_virtual_price()
                        .block(block)
                        .call()
                        .await
                        .ok()?
                }
            };
            let mut rates: Vec<U256> = decimals.iter().map(|d| rate_for_decimals(*d)).collect();
            // Override last rate with virtual_price
            if let Some(last) = rates.last_mut() {
                *last = vp;
            }
            let amp = raw_a * a_prec_100;
            Some((
                Pool::StableSwapMeta {
                    balances,
                    rates,
                    amp,
                    fee,
                },
                n_coins,
            ))
        }
        "TwoCryptoV1" | "TwoCryptoNG" => {
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
            let precisions: [U256; 2] = [
                precision_for_decimals(decimals[0]),
                precision_for_decimals(decimals[1]),
            ];
            let balances = [b0, b1];
            if entry.variant == "TwoCryptoV1" {
                Some((
                    Pool::TwoCryptoV1 {
                        balances,
                        precisions,
                        price_scale: ps,
                        d,
                        ann,
                        gamma,
                        mid_fee,
                        out_fee,
                        fee_gamma,
                    },
                    n_coins,
                ))
            } else {
                // Fuzz tests compare against on-chain get_dy (view function).
                // Both v2.0.0 and v2.1.0 get_dy use V1 fee via pool.fee_calc().
                // Note: actual swaps (_exchange) on v2.1.0 use NG fee via pool._fee().
                Some((
                    Pool::TwoCryptoNG {
                        balances,
                        precisions,
                        price_scale: ps,
                        d,
                        ann,
                        gamma,
                        mid_fee,
                        out_fee,
                        fee_gamma,
                    },
                    n_coins,
                ))
            }
        }
        "TwoCryptoStable" => {
            let c = ICrypto2::new(addr, provider);
            let b0 = c.balances(U256::from(0)).block(block).call().await.ok()?;
            let b1 = c.balances(U256::from(1)).block(block).call().await.ok()?;
            let ann = c.A().block(block).call().await.ok()?;
            let d = c.D().block(block).call().await.ok()?;
            let ps = c.price_scale().block(block).call().await.ok()?;
            let mid_fee = c.mid_fee().block(block).call().await.ok()?;
            let out_fee = c.out_fee().block(block).call().await.ok()?;
            let fee_gamma = c.fee_gamma().block(block).call().await.ok()?;
            let precisions: [U256; 2] = [
                precision_for_decimals(decimals[0]),
                precision_for_decimals(decimals[1]),
            ];
            Some((
                Pool::TwoCryptoStable {
                    balances: [b0, b1],
                    precisions,
                    price_scale: ps,
                    d,
                    ann,
                    mid_fee,
                    out_fee,
                    fee_gamma,
                },
                n_coins,
            ))
        }
        "TriCryptoV1" | "TriCryptoNG" => {
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
            let precisions: [U256; 3] = [
                precision_for_decimals(decimals[0]),
                precision_for_decimals(decimals[1]),
                precision_for_decimals(decimals[2]),
            ];
            let balances = [b0, b1, b2];
            let price_scale = [ps0, ps1];
            if entry.variant == "TriCryptoV1" {
                Some((
                    Pool::TriCryptoV1 {
                        balances,
                        precisions,
                        price_scale,
                        d,
                        ann,
                        gamma,
                        mid_fee,
                        out_fee,
                        fee_gamma,
                    },
                    n_coins,
                ))
            } else {
                Some((
                    Pool::TriCryptoNG {
                        balances,
                        precisions,
                        price_scale,
                        d,
                        ann,
                        gamma,
                        mid_fee,
                        out_fee,
                        fee_gamma,
                    },
                    n_coins,
                ))
            }
        }
        _ => None,
    }
}

// ── Shared fuzz runner ───────────────────────────────────────────────────────

async fn fuzz_pools(label: &str, pools: &[PoolEntry]) {
    // Extract chain ID from label (e.g. "chain 8453 pending" → 8453)
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
    let bn = latest - 5; // use a settled block to avoid inconsistent state
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
        let (pool, n_coins) = match build_pool(entry, &provider, block).await {
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

/// Full fuzz: all pools in registry. Triggered by code changes.
#[tokio::test]
#[ignore = "requires RPC_URL_1 or RPC_URL"]
async fn fuzz_1() {
    let pools = load_registry("tests/registry/1.toml");
    fuzz_pools("chain 1", &pools).await;
}

/// Pending fuzz: only new pools. Triggered by indexer PR.
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

/// Fuzz all Base pools.
#[tokio::test]
#[ignore = "requires RPC_URL_8453"]
async fn fuzz_8453() {
    let pools = load_registry("tests/registry/8453.toml");
    fuzz_pools("chain 8453", &pools).await;
}

/// Pending fuzz for Base.
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
