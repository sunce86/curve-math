# curve-math

[![CI](https://github.com/sunce86/curve-math/actions/workflows/unit-tests.yml/badge.svg)](https://github.com/sunce86/curve-math/actions/workflows/unit-tests.yml)
[![crates.io](https://img.shields.io/crates/v/curve-math.svg)](https://crates.io/crates/curve-math)
[![docs.rs](https://docs.rs/curve-math/badge.svg)](https://docs.rs/curve-math)

Pure Rust implementation of [Curve Finance](https://curve.fi/) AMM math. Exact on-chain match — no tolerances, no approximations, wei-level precision.

## Verified Pools

Every registered pool is **differentially fuzz-tested** against its on-chain `get_dy`: our `get_amount_out` is called with random swap amounts and the result is compared with an on-chain call at the same block. The test requires **exact wei-level match** — no tolerances, no approximations. Pools that don't match are not registered.

The pool registry covers all Curve factory pools above ~$1K TVL and all legacy (pre-factory) pools above ~$5K TVL.

| Chain | Fuzz | Verified pools | Last indexed |
|-------|------|----------------|-------------|
| Ethereum | [![Fuzz](https://github.com/sunce86/curve-math/actions/workflows/fuzz-ethereum.yml/badge.svg)](https://github.com/sunce86/curve-math/actions/workflows/fuzz-ethereum.yml) | 209 / 1227 ![](https://geps.dev/progress/17?successColor=6366f1) | 2026-03-22 |

## Performance

Compared against [revm](https://github.com/bluealloy/revm) executing the same pool's on-chain `get_dy` bytecode. Both produce identical results (wei-exact).

**Pure computation** — revm with pre-loaded state (EVM interpretation overhead only):

| Pool type | curve-math | revm | Speedup |
|-----------|-----------|------|---------|
| StableSwap 2-coin (3pool) | 1.6 µs | 13 µs | **8x** |
| StableSwapNG with oracle rates (sUSDS/USDT) | 1.4 µs | 38 µs | **27x** |
| TwoCryptoNG Cardano cubic (crvUSD/FXN) | 6.4 µs | 34 µs | **5x** |
| TriCryptoNG 3-coin hybrid (crvUSD/WETH/CRV) | 4.5 µs | 52 µs | **12x** |

**Realistic simulation** — revm with full EVM setup (DB, accounts, storage, bytecode):

| Pool type | curve-math | revm | Speedup |
|-----------|-----------|------|---------|
| StableSwap 2-coin (3pool) | 1.6 µs | 150 µs | **94x** |
| StableSwapNG with oracle rates (sUSDS/USDT) | 1.4 µs | 448 µs | **320x** |
| TwoCryptoNG Cardano cubic (crvUSD/FXN) | 6.4 µs | 313 µs | **49x** |
| TriCryptoNG 3-coin hybrid (crvUSD/WETH/CRV) | 4.5 µs | 358 µs | **80x** |

<sub>MacBook M2, Rust 1.82, revm 36. Reproduce: `cd benches/revm-comparison && cargo bench`</sub>

## Coverage

All 11 Curve pool variants:

| Variant | Type | Solver | Example pools | Vyper source |
|---------|------|--------|---------------|--------------|
| `StableSwapV0` | StableSwap | Newton | sUSD, Compound, USDT, y, BUSD | [StableSwapSUSD.vy](https://github.com/curvefi/curve-contract/blob/master/contracts/pools/susd/StableSwapSUSD.vy) |
| `StableSwapV1` | StableSwap | Newton | 3pool, ren, sbtc, hbtc | [StableSwap3Pool.vy](https://github.com/curvefi/curve-contract/blob/master/contracts/pools/3pool/StableSwap3Pool.vy) |
| `StableSwapV2` | StableSwap | Newton | FRAX/USDC, stETH, factory plain | [SwapTemplateBase.vy](https://github.com/curvefi/curve-contract/blob/master/contracts/pool-templates/base/SwapTemplateBase.vy) |
| `StableSwapALend` | StableSwap | Newton | Aave, sAAVE, IB, aETH | [SwapTemplateA.vy](https://github.com/curvefi/curve-contract/blob/master/contracts/pool-templates/a/SwapTemplateA.vy) |
| `StableSwapNG` | StableSwap | Newton | StableSwap-NG (plain + meta) | [CurveStableSwapNG.vy](https://github.com/curvefi/stableswap-ng/blob/main/contracts/main/CurveStableSwapNG.vy) |
| `StableSwapMeta` | StableSwap | Newton | GUSD, HUSD, factory meta | [SwapTemplateMeta.vy](https://github.com/curvefi/curve-contract/blob/master/contracts/pool-templates/meta/SwapTemplateMeta.vy) |
| `TwoCryptoV1` | CryptoSwap | Newton | CRV/ETH (legacy) | [CurveCryptoSwap2ETH.vy](https://github.com/curvefi/curve-crypto-contract/blob/master/contracts/two/CurveCryptoSwap2ETH.vy) |
| `TwoCryptoNG` | CryptoSwap | Cardano cubic | crvUSD/FXN (MATH v2.0.0) | [TwocryptoMath.vy](https://github.com/curvefi/twocrypto-ng/blob/main/contracts/main/TwocryptoMath.vy) |
| `TwoCryptoStable` | StableSwap | Newton | crvUSD/WETH (MATH v0.1.0) | [Etherscan](https://etherscan.io/address/0x79839c2D74531A8222C0F555865aAc1834e82e51#code) |
| `TriCryptoV1` | CryptoSwap | Newton | tricrypto2 (USDT/WBTC/WETH) | [CurveCryptoMath3.vy](https://github.com/curvefi/curve-crypto-contract/blob/master/contracts/tricrypto/CurveCryptoMath3.vy) |
| `TriCryptoNG` | CryptoSwap | Hybrid cubic+Newton | tricrypto-ng (USDC/WBTC/WETH) | [CurveTricryptoOptimized.vy](https://github.com/curvefi/tricrypto-ng/blob/main/contracts/main/CurveTricryptoOptimized.vy) |

## Usage

```toml
[dependencies]
curve-math = { git = "https://github.com/sunce86/curve-math" }                    # core math only
curve-math = { git = "https://github.com/sunce86/curve-math", features = ["swap"] }  # + Pool enum
```

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

## Structure

```
src/
  core/           # Pure math — Newton solvers, Cardano cubic, fee functions
  swap/           # get_amount_out/in, spot_price per variant (feature-gated)
  pool.rs         # Pool enum — unified API over all variants (feature-gated)
registry/
  1.toml          # Verified pools on Ethereum mainnet (chain ID 1)
docs/
  integration.md  # How to integrate with an indexer or solver
```

- **`core`** (always available): Stateless math functions ported line-by-line from Vyper. Each variant file links to the exact Vyper source it was verified against.
- **`swap`** + **`Pool`** (behind `swap` feature): Pool simulation with normalization, fees, and denormalization.
- **[`docs/integration.md`](docs/integration.md)**: Variant detection, state tracking, A parameter scaling, and streaming integration guide.

## Dependencies

Only [`alloy-primitives`](https://crates.io/crates/alloy-primitives) (U256/I256). Zero runtime dependencies beyond that.

## License

MIT
