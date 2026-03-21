# curve-math

Pure Rust implementation of [Curve Finance](https://curve.fi/) AMM math. Exact on-chain match — no tolerances, no approximations, wei-level precision.

## Coverage

All 10 Curve pool variants:

| Variant | Type | Solver | Example pools |
|---------|------|--------|---------------|
| `StableSwapV0` | StableSwap | Newton | sUSD, Compound, USDT, y, BUSD |
| `StableSwapV1` | StableSwap | Newton | 3pool, ren, sbtc, hbtc |
| `StableSwapV2` | StableSwap | Newton | FRAX/USDC, stETH, factory plain |
| `StableSwapALend` | StableSwap | Newton | Aave, sAAVE, IB, aETH |
| `StableSwapNG` | StableSwap | Newton | StableSwap-NG (plain + meta) |
| `StableSwapMeta` | StableSwap | Newton | GUSD, HUSD, factory meta |
| `TwoCryptoV1` | CryptoSwap | Newton | CRV/ETH (legacy) |
| `TwoCryptoNG` | CryptoSwap | Cardano cubic | crvUSD/FXN (twocrypto-ng) |
| `TriCryptoV1` | CryptoSwap | Newton | tricrypto2 (USDT/WBTC/WETH) |
| `TriCryptoNG` | CryptoSwap | Hybrid cubic+Newton | tricrypto-ng (USDC/WBTC/WETH) |

Each variant is verified against on-chain `get_dy` with `assert_eq` (exact match, not approximate).

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

## Structure

```
src/
  core/       # Pure math — Newton solvers, Cardano cubic, fee functions
  swap/       # get_amount_out/in, spot_price per variant (feature-gated)
  pool.rs     # Pool enum — unified API over all variants (feature-gated)
```

- **`core`** (always available): Stateless math functions ported line-by-line from Vyper. No pool state, no normalization. Each variant file is self-contained — no cross-file dependencies.

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
cargo test                                           # core unit tests
cargo test --features swap                           # + swap unit tests + roundtrip tests
RPC_URL=<ethereum-mainnet> \
  cargo test --features swap -- --ignored            # + 10 on-chain verification tests
```

## Dependencies

Only [`alloy-primitives`](https://crates.io/crates/alloy-primitives) (U256/I256). Zero runtime dependencies beyond that.

## License

MIT
