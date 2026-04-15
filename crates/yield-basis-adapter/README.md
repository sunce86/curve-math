# yield-basis-adapter

Data-source agnostic pool construction for Yield Basis LEVAMM.

## What it does

- **Pool construction** — `build_pool(RawYieldBasisState, block_timestamp) -> YieldBasisPool` with interest accrual
- Immutable fields (leverage, lev_ratio, collateral_precision) from deploy-time constants
- Mutable fields (debt, collateral, fee, rate) from on-chain state

## Usage

```rust
use yield_basis_adapter::{RawYieldBasisState, build_pool};

let state = RawYieldBasisState {
    leverage, lev_ratio, collateral_precision,
    fee, collateral_amount, stored_debt,
    rate_mul, rate, rate_time, p_oracle,
};

let pool = build_pool(&state, block_timestamp)?;
let dy = pool.get_amount_out(0, 1, dx)?;
```

## Interest accrual

`build_pool` computes current debt from stored values:
- `current_rate_mul = stored_rate_mul * (1 + rate * dt) / 1e18`
- `current_debt = stored_debt * current_rate_mul / stored_rate_mul`

This matches `AMM.vy::_debt()` and `_rate_mul()`.
