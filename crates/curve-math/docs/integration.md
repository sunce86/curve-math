# Integration Guide

How to integrate `curve-math` into a solver, indexer, or routing engine.

## Recommended: use curve-adapter

[`curve-adapter`](../../curve-adapter/) handles variant detection, rate computation, and pool construction. Most consumers should use it instead of constructing `Pool` manually:

```rust
use curve_adapter::{CurveVariant, RawPoolState, build_pool};

let state = RawPoolState {
    variant: CurveVariant::StableSwapV2,
    balances: vec![bal0, bal1],
    token_decimals: vec![18, 6],
    amp,
    fee: Some(fee),
    ..Default::default()
};
let pool = build_pool(&state)?;
let dy = pool.get_amount_out(0, 1, dx);
```

See `RawPoolState` field documentation for edge cases (A() precision loss, balances vs balanceOf, rebase tokens, etc.).

## Pool Variant Detection

Every Curve pool uses one of 11 math variants. Using the wrong variant produces wrong swap amounts.

### Factory-deployed pools (NG)

Pools deployed from NG factories emit a deployment event that determines the variant.

| Factory | Event | Variant |
|---------|-------|---------|
| StableSwap-NG | `PlainPoolDeployed` / `MetaPoolDeployed` | `StableSwapNG` / `StableSwapMeta` |
| TwoCrypto-NG | `TwocryptoPoolDeployed` | See below |
| TriCrypto-NG | `TricryptoPoolDeployed` | `TriCryptoNG` |

**TwoCrypto-NG variant detection:** The deploy event contains a `math` field — the MATH contract address. Different MATH contracts implement different invariants:

| MATH version | Invariant | Variant |
|---|---|---|
| `v2.0.0` | CryptoSwap (with gamma) | `TwoCryptoNG` |
| `v2.1.0` | CryptoSwap (with gamma) | `TwoCryptoNG` |
| `v0.1.0` | StableSwap (gamma ignored) | `TwoCryptoStable` |

Known MATH addresses (Ethereum mainnet):
```
0x2005995a71243be9FB995DaB4742327dc76564Df → TwoCryptoNG  (v2.0.0)
0x1Fd8Af16DC4BEBd950521308D55d0543b6cDF4A1 → TwoCryptoNG  (v2.1.0)
0x79839c2D74531A8222C0F555865aAc1834e82e51 → TwoCryptoStable (v0.1.0)
```

### Factory-deployed pools (legacy)

| Factory | Address | Variant |
|---------|---------|---------|
| MetaPool Factory | `0xB9fC157394Af804a3578134A6585C0dc9cc990d4` | `StableSwapV2` or `StableSwapMeta` (use `is_meta()` or event type) |
| CryptoSwap Factory | `0xF18056Bbd320E96A48e3Fbf8bC061322531aac99` | `TwoCryptoV1` |
| crvUSD StableSwap Factory | `0x4F8846Ae9380B90d2E71D5e3D042dff3E7ebb40d` | `StableSwapV2` |

### Legacy pools (pre-factory)

12 pools deployed before factories existed (8 V0, 2 V1, 2 TriCryptoV1). These are a fixed set — no new ones will be created. Use `curve_adapter::detect_variant()` with on-chain probing, or hardcode from the known address lists in `detect.rs`.

### On-chain probing (unknown pools)

If you don't know the variant, use `curve_adapter::detect_variant()`:

```rust
use curve_adapter::{detect_variant, ProbingResults};

let probing = ProbingResults {
    has_gamma: /* call gamma() */,
    n_coins: /* count coins(i) calls */,
    has_math: /* call MATH() */,
    math_version: /* call MATH().version() */,
    has_offpeg_fee_multiplier: /* call offpeg_fee_multiplier() */,
    has_stored_rates: /* call stored_rates() */,
    has_base_pool: /* call base_pool() */,
    has_int128_balances: /* call balances(int128(0)) */,
    pool_address: addr,
};
let variant = detect_variant(&probing)?;
```

## Pool State: What to Read and When

### Per-block state (update on every swap/liquidity event)

| Variant | Fields |
|---------|--------|
| All StableSwap | `balances` |
| StableSwapNG (oracle tokens) | `balances`, `stored_rates()` |
| All CryptoSwap + TwoCryptoStable | `balances`, `D`, `price_scale` |

### Semi-static state (update on admin events)

| Field | When it changes | Event to watch |
|-------|----------------|----------------|
| `A` / `amp` | During A ramping (takes days) | `RampAgamma` / `StopRampA` |
| `gamma` | During gamma ramping | `RampAgamma` / `StopRampA` |

### Static state (read once at discovery)

| Variant | Fields |
|---------|--------|
| StableSwap V0/V1/V2/Meta | `fee`, rates (computed from token decimals) |
| StableSwapALend | `fee`, `offpeg_fee_multiplier` |
| StableSwapNG | `fee`, `offpeg_fee_multiplier` |
| CryptoSwap / TwoCryptoStable | `mid_fee`, `out_fee`, `fee_gamma` |

### Rate computation

For StableSwap pools, `rates` depend on token type:

| Token type | Rate | How to get |
|---|---|---|
| Plain (18-dec) | `10^18` | From decimals: `10^(36 - decimals)` |
| Plain (6-dec) | `10^30` | From decimals: `10^(36 - decimals)` |
| ERC4626 vault (sDAI, sUSDe) | Dynamic | Read `pool.stored_rates()` every block |
| Meta pool LP token (3Crv) | `virtual_price` | Read from base pool every block |

`build_pool()` computes static rates automatically from `token_decimals`. For dynamic rates, provide them via `RawPoolState::dynamic_rates`.

### A parameter scaling

| Variant | On-chain `A()` returns | Pass to `RawPoolState::amp` as |
|---|---|---|
| V0, V1 | Raw A (A_PRECISION=1) | `amp = A()` |
| V2, Meta, NG, ALend | A / A_PRECISION (lossy) | `amp = initial_A()` or `A() * 100` |
| CryptoSwap (all) | A * A_MULTIPLIER (10000) | `amp = A()` (already scaled) |

**Prefer `initial_A()`** over `A() * 100` for V2+ — `A()` loses remainder via integer division.

For storage-based consumers (Substreams), use `curve_adapter::interpolate_a()` to compute amp from `initial_A`/`future_A`/timestamps.
