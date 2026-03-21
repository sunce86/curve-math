# curve-math

Pure Rust implementation of [Curve Finance](https://curve.fi/) AMM math. Exact on-chain match — no tolerances, no approximations, wei-level precision.

## Coverage

All 10 Curve pool variants:

| Variant | Type | Solver | Example pools |
|---------|------|--------|---------------|
| StableSwapV0 | StableSwap | Newton | sUSD, Compound, USDT, y, BUSD |
| StableSwapV1 | StableSwap | Newton | 3pool, ren, sbtc, hbtc |
| StableSwapV2 | StableSwap | Newton | FRAX/USDC, stETH, factory plain |
| StableSwapALend | StableSwap | Newton | Aave, sAAVE, IB, aETH |
| StableSwapNG | StableSwap | Newton | StableSwap-NG (plain + meta) |
| StableSwapMeta | StableSwap | Newton | GUSD, HUSD, factory meta |
| TwoCryptoV1 | CryptoSwap | Newton | CRV/ETH (legacy) |
| TwoCryptoNG | CryptoSwap | Cardano cubic | crvUSD/FXN (twocrypto-ng) |
| TriCryptoV1 | CryptoSwap | Newton | tricrypto2 (USDT/WBTC/WETH) |
| TriCryptoNG | CryptoSwap | Hybrid cubic+Newton | tricrypto-ng (USDC/WBTC/WETH) |

Each variant is verified against on-chain `get_dy` with `assert_eq` (exact match, not approximate).

## Structure

```
src/
  core/       # Pure math — Newton solvers, Cardano cubic, fee functions
  swap/       # get_amount_out per variant + on-chain tests (feature-gated)
```

- **`core`** (always available): Stateless math functions ported line-by-line from Vyper. No pool state, no normalization. Each variant file is self-contained — no cross-file dependencies.

- **`swap`** (behind `swap` feature): `get_amount_out` functions that handle balance normalization, fee calculation, and denormalization. Each file imports only from its corresponding `core` module.

## Usage

```toml
[dependencies]
curve-math = { git = "https://github.com/sunce86/curve-math" }                  # core only
curve-math = { git = "https://github.com/sunce86/curve-math", features = ["swap"] }  # + get_amount_out
```

```rust
use curve_math::core::stableswap_v2::{get_d, get_y};
use curve_math::swap::stableswap_v2::get_amount_out; // requires "swap" feature
```

## Testing

```bash
cargo test                              # 38 core unit tests
cargo test --features swap              # + 3 swap unit tests
RPC_URL=<ethereum-mainnet> \
  cargo test --features swap -- --ignored  # + 10 on-chain verification tests
```

## Dependencies

Only [`alloy-primitives`](https://crates.io/crates/alloy-primitives) (U256/I256). Zero runtime dependencies beyond that.

## License

MIT
