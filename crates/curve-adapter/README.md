# curve-adapter

Adapts raw on-chain Curve pool data into [`curve-math::Pool`](https://crates.io/crates/curve-math) instances ready for swap computation.

## What it does

- **Variant detection** — classifies pools into one of 11 `CurveVariant`s via on-chain probing
- **Pool construction** — `build_pool(RawPoolState) → Pool` with rate/precision computation and validation

## Usage

```rust
use curve_adapter::{CurveVariant, RawPoolState, build_pool};
use alloy_primitives::U256;

let state = RawPoolState {
    variant: CurveVariant::StableSwapV2,
    balances: vec![U256::from(1_000_000_000_000_000_000_000u128),
                   U256::from(1_000_000_000_000u128)],
    token_decimals: vec![18, 6],
    amp: U256::from(40_000u64), // A=400 * A_PRECISION=100
    fee: Some(U256::from(4_000_000u64)),
    ..Default::default()
};

let pool = build_pool(&state).unwrap();
let dy = pool.get_amount_out(0, 1, U256::from(1_000_000_000_000_000_000u128));
```

## Who computes `amp`?

`build_pool()` expects an already-interpolated amplification parameter. How you get it depends on your transport:

- **RPC** — call `A()` on the pool contract
- **Substreams / storage** — read `initial_A`/`future_A`/timestamps, then use `curve_adapter::interpolate_a()`
