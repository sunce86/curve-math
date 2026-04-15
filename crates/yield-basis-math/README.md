# yield-basis-math

Pure Rust port of [Yield Basis](https://github.com/yield-basis/yb-core) LEVAMM math.
Wei-level precision, fuzz-verified against on-chain contracts.

## What it does

Constant-product AMM with leverage, where the "x" side is derived from oracle price and debt via a quadratic solver (`get_x0`). Supports both directions: buy collateral (stablecoin in) and sell collateral (collateral in).

## Usage

```rust
use yield_basis_math::pool::YieldBasisPool;

let pool = YieldBasisPool::new(
    leverage, lev_ratio, collateral_precision,
    fee, collateral_amount, debt, p_oracle,
)?;

let dy = pool.get_amount_out(0, 1, dx)?; // buy collateral
let price = pool.get_amount_out(1, 0, dx)?; // sell collateral
```

## Architecture

- `constants` — `WAD`, `MAX_FEE`
- `core` — stateless math: `get_x0` (quadratic), `sqrt`, `ceil_div`, `compute_rate_mul`, `compute_debt`
- `swap` + `pool` — `YieldBasisPool` with `get_amount_out` (requires `swap` feature)
