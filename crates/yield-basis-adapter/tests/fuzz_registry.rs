//! Registry-driven differential fuzz test for Yield Basis LEVAMM.
//!
//! Reads `tests/registry/1.toml`, builds each pool via `build_pool`,
//! and compares `get_amount_out` against on-chain `get_dy` (wei-exact).
//!
//! Run:
//!   FUZZ_ITERATIONS=20 RPC_URL_1=... \
//!     cargo test -p yield-basis-adapter --test fuzz_registry -- --ignored --nocapture

use alloy::providers::{Provider, ProviderBuilder};
use alloy_primitives::{Address, U256};
use serde::Deserialize;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use yield_basis_adapter::{build_pool, RawYieldBasisState};

#[derive(Deserialize)]
struct Registry {
    pools: Vec<PoolEntry>,
}

#[derive(Deserialize, Clone)]
struct PoolEntry {
    amm: String,
    name: String,
    leverage: u128,
    lev_ratio: u128,
    collateral_precision: u64,
}

alloy::sol! {
    #[sol(rpc)]
    interface IAMM {
        function get_dy(uint256 i, uint256 j, uint256 in_amount) external view returns (uint256);
        function fee() external view returns (uint256);
        function collateral_amount() external view returns (uint256);
        function debt() external view returns (uint256);
        function rate() external view returns (uint256);
        function rate_mul() external view returns (uint256);
    }

    #[sol(rpc)]
    interface IPriceOracle {
        function price() external view returns (uint256);
    }
}

fn fuzz_iterations() -> usize {
    std::env::var("FUZZ_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10)
}

fn splitmix64(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = *seed;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

fn generate_amounts(n: usize, max: U256, seed: u64) -> Vec<U256> {
    let mut amounts = vec![U256::from(1u64)];
    if !max.is_zero() {
        amounts.extend_from_slice(&[
            max / U256::from(1000u64),
            max / U256::from(10u64),
            max / U256::from(2u64),
            max,
        ]);
    }
    let remaining = n.saturating_sub(amounts.len());
    if remaining > 0 && !max.is_zero() {
        let mut seed = seed;
        let max_f64 = max.to_string().parse::<f64>().unwrap_or(1e30);
        let ln_max = max_f64.ln();
        for _ in 0..remaining {
            let r = splitmix64(&mut seed);
            let t = (r as f64) / (u64::MAX as f64);
            let val = (t * ln_max).exp();
            let val_u128 = val.min(1e38) as u128;
            amounts.push(U256::from(val_u128).max(U256::from(1u64)).min(max));
        }
    }
    amounts
}

// TODO: Add read_state function when pool addresses are known.
// Need to determine: PRICE_ORACLE_CONTRACT address, rate_time storage slot,
// stored_debt vs accrued debt distinction.

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "requires RPC_URL"]
async fn fuzz_1() {
    let rpc_url = std::env::var("RPC_URL_1").expect("RPC_URL_1 must be set");
    let _provider = Arc::new(ProviderBuilder::new().connect_http(rpc_url.parse().unwrap()));
    let registry_toml = include_str!("registry/1.toml");
    let registry: Registry = toml::from_str(registry_toml).expect("invalid registry TOML");

    // TODO: implement fuzz loop once pool addresses are populated
    eprintln!(
        "yield-basis fuzz: {} pools registered, awaiting implementation",
        registry.pools.len()
    );
}
