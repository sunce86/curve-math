# Integration Guide

How to integrate `curve-math` into a solver, indexer, or routing engine.

## Pool Variant Detection

Every Curve pool uses one of 11 math variants. Using the wrong variant produces wrong swap amounts. Detection depends on how the pool was deployed.

### Factory-deployed pools (NG)

Pools deployed from NG factories emit a deployment event that contains all information needed to determine the variant. No on-chain probing required.

| Factory | Event | Variant |
|---------|-------|---------|
| StableSwap-NG | `PlainPoolDeployed` / `MetaPoolDeployed` | `StableSwapNG` |
| TwoCrypto-NG | `TwocryptoPoolDeployed` | See below |
| TriCrypto-NG | `TricryptoPoolDeployed` | `TriCryptoNG` |

**TwoCrypto-NG variant detection:** The `TwocryptoPoolDeployed` event contains a `math` field — the address of the MATH contract used by the pool. Different MATH contracts implement different invariants:

| MATH version | Invariant | Variant | How to detect |
|---|---|---|---|
| `v2.0.0` | CryptoSwap (with gamma) | `TwoCryptoNG` | `math.version() == "v2.0.0"` |
| `v2.1.0` | CryptoSwap (with gamma) | `TwoCryptoNG` | `math.version() == "v2.1.0"` |
| `v0.1.0` | StableSwap (gamma ignored) | `TwoCryptoStable` | `math.version() == "v0.1.0"` |

Since there are only a few MATH contract addresses per chain, you can maintain a small lookup table instead of calling `version()` at runtime:

```
# Ethereum mainnet MATH addresses
0x2005995a71243be9FB995DaB4742327dc76564Df → TwoCryptoNG  (v2.0.0)
0x1Fd8Af16DC4BEBd950521308D55d0543b6cDF4A1 → TwoCryptoNG  (v2.1.0)
0x79839c2D74531A8222C0F555865aAc1834e82e51 → TwoCryptoStable (v0.1.0)
```

### Factory-deployed pools (legacy)

Three legacy factories also use the same `pool_count()` / `pool_list(uint256)` interface as NG factories. The variant is either fixed per factory or detected on-chain.

| Factory | Address | Variant |
|---------|---------|---------|
| MetaPool Factory | `0xB9fC157394Af804a3578134A6585C0dc9cc990d4` | `StableSwapMeta` |
| CryptoSwap Factory | `0xF18056Bbd320E96A48e3Fbf8bC061322531aac99` | `TwoCryptoV1` |
| crvUSD StableSwap Factory | `0x4F8846Ae9380B90d2E71D5e3D042dff3E7ebb40d` | Probe on-chain (see `detect_variant.py`) |

The [pool indexer](../tools/index-pools.py) discovers pools from all 6 factories (3 NG + 3 legacy).

### Legacy pools (pre-factory)

Legacy pools were deployed individually before factories existed. There is no deployment event to parse. Use the [verified pool registry](../registry/) as the source of truth:

```toml
# registry/1.toml
[[pools]]
address = "0xbEbc44782C7dB0a1A60Cb6fe97d0b483032FF1C7"
variant = "StableSwapV1"
name = "3pool (DAI/USDC/USDT)"
```

The registry is maintained by the [pool indexer](../tools/index-pools.py) and covers all legacy pools with meaningful liquidity. Legacy pools are a fixed set — no new ones are being deployed.

## Pool State: What to Read and When

Each variant requires different on-chain state. Some fields change every block, others rarely.

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
| `fee` | Admin action (rare) | `NewFee` / `ApplyNewFee` |

### Static state (read once at discovery)

| Variant | Fields |
|---------|--------|
| StableSwap V0/V1/V2/Meta | `fee`, rates (computed from token decimals) |
| StableSwapALend | `fee`, `offpeg_fee_multiplier`, `precision_mul` |
| StableSwapNG | `fee`, `offpeg_fee_multiplier`, rates (or `stored_rates()` for oracle) |
| CryptoSwap / TwoCryptoStable | `mid_fee`, `out_fee`, `fee_gamma`, `precisions` |

### Rate computation

For StableSwap pools, `rates` depend on token type:

| Token type | Rate | How to get |
|---|---|---|
| Plain (18-dec) | `10^18` | From decimals: `10^(36 - decimals)` |
| Plain (6-dec) | `10^30` | From decimals: `10^(36 - decimals)` |
| ERC4626 vault (sDAI, sUSDe) | Dynamic | Read `pool.stored_rates()` every block |
| Meta pool LP token (3Crv) | `virtual_price` | Read from base pool |

Pools with `oracle_rates = true` in the registry use dynamic rates.

## Constructing a Pool

Once you know the variant and have the state, construct the `Pool` enum directly:

```rust
use curve_math::Pool;

// StableSwap example
let pool = Pool::StableSwapNG {
    balances: vec![bal0, bal1],
    rates: vec![rate0, rate1],  // from stored_rates() or computed
    amp,                         // A() * A_PRECISION for V2/NG/Meta
    fee,
    offpeg_fee_multiplier,
};

// CryptoSwap example
let pool = Pool::TwoCryptoNG {
    balances: [b0, b1],
    precisions: [prec0, prec1],  // 10^(18 - decimals)
    price_scale,
    d,                            // from pool.D()
    ann,                          // from pool.A() (already includes A_MULTIPLIER)
    gamma,
    mid_fee, out_fee, fee_gamma,
};

// TwoCryptoStable (StableSwap math with CryptoSwap interface)
let pool = Pool::TwoCryptoStable {
    balances: [b0, b1],
    precisions: [prec0, prec1],
    price_scale,
    d,                            // from pool.D()
    ann,                          // from pool.A()
    mid_fee, out_fee, fee_gamma,
};

let dy = pool.get_amount_out(0, 1, dx)?;
```

### A parameter scaling

Different variants scale A differently:

| Variant | On-chain `A()` returns | Pass to Pool as |
|---|---|---|
| V0, V1 | Raw A | `amp = A()` |
| V2, Meta, NG, ALend | Raw A (needs A_PRECISION=100) | `amp = A() * 100` |
| TwoCryptoV1, TwoCryptoNG, TwoCryptoStable | A * A_MULTIPLIER (10000) | `ann = A()` (already scaled) |
| TriCryptoV1, TriCryptoNG | A * A_MULTIPLIER (10000) | `ann = A()` (already scaled) |

## Substreams / Streaming Integration

For streaming architectures that cannot make `eth_call`:

1. **Discovery:** Catch factory deployment events. Variant is determined from event data (factory address + MATH address for TwoCrypto).

2. **Legacy pools:** Load `registry/<chain_id>.toml` at startup. This is a static file — legacy pools never change.

3. **State tracking:** Monitor storage slot changes for the fields listed above. The pool contract's storage layout determines which slots map to `balances`, `D`, `price_scale`, etc.

4. **Delta application:** On each block, apply storage deltas to your cached Pool state, then call `get_amount_out` for pricing.
