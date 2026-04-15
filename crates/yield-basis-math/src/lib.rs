//! Pure Rust port of [Yield Basis](https://github.com/yield-basis/yb-core)
//! LEVAMM math. Wei-level precision, fuzz-verified against on-chain contracts.
//!
//! # Architecture
//!
//! - **`core`** — stateless math (`get_x0`, `sqrt`, `ceil_div`).
//!   Always available, zero deps beyond `alloy-primitives`.
//! - **`swap`** + **`pool`** — `YieldBasisPool` with `get_amount_out`.
//!   Requires the `swap` feature.
//!
//! # Quick start
//!
//! ```rust,ignore
//! use yield_basis_math::pool::YieldBasisPool;
//!
//! let pool = YieldBasisPool::new(
//!     leverage, lev_ratio, collateral_precision,
//!     fee, collateral_amount, debt, p_oracle,
//! )?;
//!
//! let dy = pool.get_amount_out(0, 1, dx)?; // buy collateral
//! ```
//!
//! Ported from [`AMM.vy`](https://github.com/yield-basis/yb-core/blob/main/contracts/AMM.vy).

pub mod constants;
pub mod core;

#[cfg(feature = "swap")]
pub mod swap;

#[cfg(feature = "swap")]
pub mod pool;
