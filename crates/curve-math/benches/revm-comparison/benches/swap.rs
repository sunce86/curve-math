//! Benchmark: curve-math get_amount_out vs revm executing on-chain get_dy.
//!
//! Both produce identical results (wei-exact). We measure pure computation time.
//!
//! Run: cd benches/revm-comparison && cargo bench

use alloy_primitives::{Address, Bytes, U256};
use criterion::{criterion_group, criterion_main, Criterion};
use curve_math::Pool;
use revm::{
    bytecode::Bytecode, context::TxEnv, database::CacheDB, database_interface::EmptyDB,
    primitives::TxKind, state::AccountInfo, Context, ExecuteEvm, MainBuilder, MainContext,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::str::FromStr;

// ── Fixture types ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Fixture {
    name: String,
    variant: String,
    pool_address: String,
    calldata: String,
    i: usize,
    j: usize,
    dx: String,
    expected_dy: String,
    pool_params: PoolParams,
    accounts: HashMap<String, AccountState>,
}

#[derive(Deserialize)]
struct PoolParams {
    #[serde(rename = "A")]
    a: String,
    n_coins: usize,
    decimals: Vec<u8>,
    balances: Vec<String>,
    #[serde(default)]
    fee: Option<String>,
    #[serde(default)]
    rates: Option<Vec<String>>,
    #[serde(default)]
    offpeg_fee_multiplier: Option<String>,
    #[serde(default)]
    gamma: Option<String>,
    #[serde(rename = "D", default)]
    d: Option<String>,
    #[serde(default)]
    price_scale: Option<serde_json::Value>, // String or [String, String]
    #[serde(default)]
    mid_fee: Option<String>,
    #[serde(default)]
    out_fee: Option<String>,
    #[serde(default)]
    fee_gamma: Option<String>,
    #[serde(default)]
    precisions: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct AccountState {
    code: String,
    storage: HashMap<String, String>,
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn u(s: &str) -> U256 {
    U256::from_str(s).unwrap()
}

fn load_fixture(name: &str) -> Fixture {
    let path = format!("{}/fixtures/{}.json", env!("CARGO_MANIFEST_DIR"), name);
    let data = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Fixture not found: {path}. Run generate_fixtures.py first."));
    serde_json::from_str(&data).expect("Invalid fixture JSON")
}

fn setup_revm_db(fixture: &Fixture) -> CacheDB<EmptyDB> {
    let mut db = CacheDB::new(EmptyDB::default());

    for (addr_hex, state) in &fixture.accounts {
        let addr = Address::from_str(addr_hex).unwrap();
        let code_bytes = hex::decode(state.code.strip_prefix("0x").unwrap_or(&state.code)).unwrap();
        let bytecode = Bytecode::new_raw(Bytes::from(code_bytes));

        let info = AccountInfo {
            balance: U256::ZERO,
            nonce: 1,
            code_hash: bytecode.hash_slow(),
            code: Some(bytecode),
            account_id: None,
        };
        db.insert_account_info(addr, info);

        for (slot_hex, val_hex) in &state.storage {
            let slot = U256::from_str(slot_hex).unwrap();
            let val = U256::from_str(val_hex).unwrap();
            let _ = db.insert_account_storage(addr, slot, val);
        }
    }

    // Insert caller account
    let caller = Address::from_str("0x0000000000000000000000000000000000000001").unwrap();
    db.insert_account_info(
        caller,
        AccountInfo {
            balance: U256::from(1_000_000_000_000_000_000u128),
            nonce: 0,
            code_hash: Default::default(),
            code: None,
            account_id: None,
        },
    );

    db
}

fn build_pool(fixture: &Fixture) -> Pool {
    let p = &fixture.pool_params;
    let bals: Vec<U256> = p.balances.iter().map(|s| u(s)).collect();

    match fixture.variant.as_str() {
        "StableSwapV1" => Pool::StableSwapV1 {
            balances: bals,
            rates: p.rates.as_ref().unwrap().iter().map(|s| u(s)).collect(),
            amp: u(&p.a),
            fee: u(p.fee.as_ref().unwrap()),
        },
        "StableSwapNG" => Pool::StableSwapNG {
            balances: bals,
            rates: p.rates.as_ref().unwrap().iter().map(|s| u(s)).collect(),
            amp: u(&p.a),
            fee: u(p.fee.as_ref().unwrap()),
            offpeg_fee_multiplier: u(p.offpeg_fee_multiplier.as_ref().unwrap()),
        },
        "TwoCryptoNG" => {
            let prec: Vec<U256> = p
                .precisions
                .as_ref()
                .unwrap()
                .iter()
                .map(|s| u(s))
                .collect();
            Pool::TwoCryptoNG {
                balances: [bals[0], bals[1]],
                precisions: [prec[0], prec[1]],
                price_scale: u(p.price_scale.as_ref().unwrap().as_str().unwrap()),
                d: u(p.d.as_ref().unwrap()),
                ann: u(&p.a),
                gamma: u(p.gamma.as_ref().unwrap()),
                mid_fee: u(p.mid_fee.as_ref().unwrap()),
                out_fee: u(p.out_fee.as_ref().unwrap()),
                fee_gamma: u(p.fee_gamma.as_ref().unwrap()),
            }
        }
        "TriCryptoNG" => {
            let prec: Vec<U256> = p
                .precisions
                .as_ref()
                .unwrap()
                .iter()
                .map(|s| u(s))
                .collect();
            let ps = p.price_scale.as_ref().unwrap().as_array().unwrap();
            Pool::TriCryptoNG {
                balances: [bals[0], bals[1], bals[2]],
                precisions: [prec[0], prec[1], prec[2]],
                price_scale: [u(ps[0].as_str().unwrap()), u(ps[1].as_str().unwrap())],
                d: u(p.d.as_ref().unwrap()),
                ann: u(&p.a),
                gamma: u(p.gamma.as_ref().unwrap()),
                mid_fee: u(p.mid_fee.as_ref().unwrap()),
                out_fee: u(p.out_fee.as_ref().unwrap()),
                fee_gamma: u(p.fee_gamma.as_ref().unwrap()),
            }
        }
        v => panic!("Unsupported variant: {v}"),
    }
}

// ── Benchmark ───────────────────────────────────────────────────────────────

fn bench_fixture(c: &mut Criterion, name: &str) {
    let fixture = load_fixture(name);
    let expected_dy = u(&fixture.expected_dy);
    let dx = u(&fixture.dx);
    let pool = build_pool(&fixture);

    // Verify curve-math matches expected
    let our_dy = pool
        .get_amount_out(fixture.i, fixture.j, dx)
        .expect("curve-math should produce result");
    assert_eq!(
        our_dy, expected_dy,
        "{}: curve-math dy={our_dy} != expected={expected_dy}",
        fixture.name
    );

    let pool_addr = Address::from_str(&fixture.pool_address).unwrap();
    let calldata = hex::decode(
        fixture
            .calldata
            .strip_prefix("0x")
            .unwrap_or(&fixture.calldata),
    )
    .unwrap();
    let caller = Address::from_str("0x0000000000000000000000000000000000000001").unwrap();

    let mut group = c.benchmark_group(&fixture.name);

    let tx = TxEnv {
        caller,
        kind: TxKind::Call(pool_addr),
        data: Bytes::from(calldata),
        value: U256::ZERO,
        gas_limit: 1_000_000,
        ..Default::default()
    };

    // revm (pure): only EVM execution, DB pre-loaded
    group.bench_function("revm (pure)", |b| {
        b.iter_batched(
            || setup_revm_db(&fixture),
            |db| {
                let ctx = Context::mainnet().with_db(db);
                let mut evm = ctx.build_mainnet();
                let result = evm.transact(tx.clone()).unwrap();
                std::hint::black_box(result);
            },
            criterion::BatchSize::SmallInput,
        );
    });

    // revm (full): includes DB setup — realistic simulation cost
    group.bench_function("revm (full)", |b| {
        b.iter(|| {
            let db = setup_revm_db(&fixture);
            let ctx = Context::mainnet().with_db(db);
            let mut evm = ctx.build_mainnet();
            let result = evm.transact(tx.clone()).unwrap();
            std::hint::black_box(result);
        });
    });

    // curve-math benchmark
    let pool_clone = pool.clone();
    let i = fixture.i;
    let j = fixture.j;
    group.bench_function("curve-math", |b| {
        b.iter(|| {
            let result = pool_clone.get_amount_out(i, j, dx);
            std::hint::black_box(result);
        });
    });

    group.finish();
}

fn bench_3pool(c: &mut Criterion) {
    bench_fixture(c, "3pool");
}
fn bench_susds_usdt(c: &mut Criterion) {
    bench_fixture(c, "sUSDS_USDT");
}
fn bench_crvusd_fxn(c: &mut Criterion) {
    bench_fixture(c, "crvUSD_FXN");
}
fn bench_crvusd_weth_crv(c: &mut Criterion) {
    bench_fixture(c, "crvUSD_WETH_CRV");
}

criterion_group!(
    benches,
    bench_3pool,
    bench_susds_usdt,
    bench_crvusd_fxn,
    bench_crvusd_weth_crv
);
criterion_main!(benches);
