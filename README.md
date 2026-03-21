# curve-math

[![Unit Tests](https://github.com/sunce86/curve-math/actions/workflows/unit-tests.yml/badge.svg)](https://github.com/sunce86/curve-math/actions/workflows/unit-tests.yml)
[![Fuzz Ethereum](https://github.com/sunce86/curve-math/actions/workflows/fuzz-ethereum.yml/badge.svg)](https://github.com/sunce86/curve-math/actions/workflows/fuzz-ethereum.yml)
![verified pools](https://img.shields.io/badge/verified%20pools-15-brightgreen)

Pure Rust implementation of [Curve Finance](https://curve.fi/) AMM math. Exact on-chain match — no tolerances, no approximations, wei-level precision.

## Coverage

All 10 Curve pool variants:

| Variant | Type | Solver | Example pools | Vyper source |
|---------|------|--------|---------------|--------------|
| `StableSwapV0` | StableSwap | Newton | sUSD, Compound, USDT, y, BUSD | [StableSwapSUSD.vy](https://github.com/curvefi/curve-contract/blob/master/contracts/pools/susd/StableSwapSUSD.vy) |
| `StableSwapV1` | StableSwap | Newton | 3pool, ren, sbtc, hbtc | [StableSwap3Pool.vy](https://github.com/curvefi/curve-contract/blob/master/contracts/pools/3pool/StableSwap3Pool.vy) |
| `StableSwapV2` | StableSwap | Newton | FRAX/USDC, stETH, factory plain | [SwapTemplateBase.vy](https://github.com/curvefi/curve-contract/blob/master/contracts/pool-templates/base/SwapTemplateBase.vy) |
| `StableSwapALend` | StableSwap | Newton | Aave, sAAVE, IB, aETH | [SwapTemplateA.vy](https://github.com/curvefi/curve-contract/blob/master/contracts/pool-templates/a/SwapTemplateA.vy) |
| `StableSwapNG` | StableSwap | Newton | StableSwap-NG (plain + meta) | [CurveStableSwapNG.vy](https://github.com/curvefi/stableswap-ng/blob/main/contracts/main/CurveStableSwapNG.vy) |
| `StableSwapMeta` | StableSwap | Newton | GUSD, HUSD, factory meta | [SwapTemplateMeta.vy](https://github.com/curvefi/curve-contract/blob/master/contracts/pool-templates/meta/SwapTemplateMeta.vy) |
| `TwoCryptoV1` | CryptoSwap | Newton | CRV/ETH (legacy) | [CurveCryptoSwap2ETH.vy](https://github.com/curvefi/curve-crypto-contract/blob/master/contracts/two/CurveCryptoSwap2ETH.vy) |
| `TwoCryptoNG` | CryptoSwap | Cardano cubic | crvUSD/FXN (twocrypto-ng) | [Twocrypto.vy](https://github.com/curvefi/twocrypto-ng/blob/main/contracts/main/Twocrypto.vy) |
| `TriCryptoV1` | CryptoSwap | Newton | tricrypto2 (USDT/WBTC/WETH) | [CurveCryptoMath3.vy](https://github.com/curvefi/curve-crypto-contract/blob/master/contracts/tricrypto/CurveCryptoMath3.vy) |
| `TriCryptoNG` | CryptoSwap | Hybrid cubic+Newton | tricrypto-ng (USDC/WBTC/WETH) | [CurveTricryptoOptimized.vy](https://github.com/curvefi/tricrypto-ng/blob/main/contracts/main/CurveTricryptoOptimized.vy) |

## Verification

Every variant is verified at three levels:

| Level | What | How |
|-------|------|-----|
| **Unit tests** | Core math (get_d, get_y, newton_y) and swap logic (roundtrip, monotonicity, spot_price) | `cargo test --features swap` — 48 tests |
| **On-chain differential fuzz** | Random swap amounts compared with `assert_eq` against deployed contracts on Ethereum mainnet | `cargo test --features swap --test fuzz_differential -- --ignored` — 10 pools × 100+ random inputs each |
| **Edge case coverage** | Fuzz includes 0, 1 wei, 0.1%/10%/50%/100%/200% of balance, and U256::MAX | Logarithmically-spaced + boundary values |

The differential fuzz tests use a deterministic PRNG seeded from the block number for reproducibility. Each test reads live pool state, generates random swap amounts across several orders of magnitude, and asserts **exact wei-level match** with the on-chain `get_dy` result. No tolerances.

## Identifying the right variant

If you know the **factory** that deployed the pool, the variant follows directly:

| Factory | Variant |
|---------|---------|
| StableSwap-NG factory | `StableSwapNG` |
| twocrypto-ng factory | `TwoCryptoNG` |
| tricrypto-ng factory | `TriCryptoNG` |
| Meta pool factory | `StableSwapMeta` |
| Plain pool factory (base template) | `StableSwapV2` |

For **legacy pools** (deployed before factories):
- Has `gamma()`? &rarr; CryptoSwap. 2-coin = `TwoCryptoV1`, 3-coin = `TriCryptoV1`
- Has `offpeg_fee_multiplier()`? &rarr; `StableSwapALend`
- `A()` returns raw A (no A_PRECISION scaling)? &rarr; `StableSwapV0` or `StableSwapV1`
  - Pool subtracts 1 from dy before denormalize? &rarr; `StableSwapV1` (3pool, ren, sbtc, hbtc)
  - Otherwise &rarr; `StableSwapV0` (sUSD, Compound, USDT, y, BUSD)

## Verified Pool Registry

The [`registry/`](registry/) directory contains TOML files listing pools that have been **fuzz-verified** against their on-chain contracts. Each pool entry includes the address, variant, token decimals, and verification status.

A pool is marked `fuzz_verified = true` only if it passes 100+ random differential swaps with `assert_eq` against on-chain `get_dy`.

Files are named by chain ID: [`registry/1.toml`](registry/1.toml) (Ethereum), `registry/42161.toml` (Arbitrum), etc.

## Structure

```
src/
  core/           # Pure math — Newton solvers, Cardano cubic, fee functions
  swap/           # get_amount_out/in, spot_price per variant (feature-gated)
  pool.rs         # Pool enum — unified API over all variants (feature-gated)
registry/
  1.toml          # Verified pools on Ethereum mainnet (chain ID 1)
tests/
  fuzz_registry.rs      # Generic registry-driven fuzz test (one test, all pools)
  fuzz_differential.rs  # Per-variant fuzz tests
  fuzz_properties.rs    # Property-based tests (roundtrip, spot_price consistency)
```

- **`core`** (always available): Stateless math functions ported line-by-line from Vyper. No pool state, no normalization. Each variant file is self-contained — no cross-file dependencies. Every file links to the exact Vyper source it was verified against.

- **`swap`** + **`Pool`** (behind `swap` feature): Pool simulation with normalization, fees, and denormalization. Use the `Pool` enum for a unified interface, or call variant-specific functions directly.

## Usage

```toml
[dependencies]
curve-math = { git = "https://github.com/sunce86/curve-math" }                    # core math only
curve-math = { git = "https://github.com/sunce86/curve-math", features = ["swap"] }  # + Pool enum
```

### With Pool enum (recommended)

```rust
use curve_math::Pool;

let pool = Pool::StableSwapV2 {
    balances: vec![bal0, bal1],
    rates: vec![rate0, rate1],
    amp,
    fee,
};

let amount_out = pool.get_amount_out(0, 1, dx)?;
let amount_in = pool.get_amount_in(0, 1, desired_dy)?;
let (price_num, price_den) = pool.spot_price(0, 1)?;
```

### Direct function calls

```rust
use curve_math::swap::stableswap_v2::{get_amount_out, get_amount_in, spot_price};

let dy = get_amount_out(&balances, &rates, amp, fee, 0, 1, dx)?;
```

### Core math only

```rust
use curve_math::core::stableswap_v2::{get_d, get_y};

let d = get_d(&xp, amp)?;
let y = get_y(0, 1, x_new, &xp, d, amp)?;
```

## Testing

```bash
# Unit + property tests (no network required, <1s)
cargo test --features swap

# Registry fuzz — verify ALL pools on a chain (requires RPC)
FUZZ_ITERATIONS=100 RPC_URL_1=<ethereum-rpc> \
  cargo test --features swap --test fuzz_registry -- fuzz_1 --ignored --nocapture

# Per-variant fuzz (for debugging a specific variant)
FUZZ_ITERATIONS=500 RPC_URL_1=<ethereum-rpc> \
  cargo test --features swap --test fuzz_differential -- fuzz_stableswap_v2 --ignored
```

CI runs unit tests on every push. Registry fuzz runs on merge to master (requires `RPC_URL_1` secret).

## Dependencies

Only [`alloy-primitives`](https://crates.io/crates/alloy-primitives) (U256/I256). Zero runtime dependencies beyond that.

## License

MIT
