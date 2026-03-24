//! Pure Rust implementation of [Curve Finance](https://curve.fi/) AMM math.
//!
//! Exact on-chain match — no tolerances, no approximations, wei-level precision.
//! Differentially fuzz-tested against on-chain `get_dy` for 200+ pools.
//!
//! # Architecture
//!
//! - **`core`** — stateless math functions (Newton solvers, Cardano cubic, fee).
//!   Always available, zero dependencies beyond `alloy-primitives`.
//! - **`swap`** + **`Pool`** — pool simulation with normalization, fees,
//!   and denormalization. Requires the `swap` feature.
//!
//! # Quick start
//!
//! ```rust,ignore
//! use curve_math::Pool; // requires feature "swap"
//! use alloy_primitives::U256;
//!
//! let pool = Pool::StableSwapV2 {
//!     balances: vec![U256::from(1_000_000_000_000_000_000_000u128),
//!                    U256::from(1_000_000_000_000_000_000_000u128)],
//!     rates: vec![U256::from(1_000_000_000_000_000_000u128),
//!                 U256::from(1_000_000_000_000_000_000u128)],
//!     amp: U256::from(40_000u64),    // A * A_PRECISION (400 * 100)
//!     fee: U256::from(4_000_000u64), // 0.04%
//! };
//!
//! let dx = U256::from(1_000_000_000_000_000_000u128); // 1 token
//! let dy = pool.get_amount_out(0, 1, dx).expect("swap should succeed");
//! assert!(dy > U256::ZERO);
//! ```
//!
//! # Supported variants
//!
//! All 11 Curve pool types: `StableSwapV0`, `StableSwapV1`, `StableSwapV2`,
//! `StableSwapALend`, `StableSwapNG`, `StableSwapMeta`, `TwoCryptoV1`,
//! `TwoCryptoNG`, `TwoCryptoStable`, `TriCryptoV1`, `TriCryptoNG`.

#![allow(clippy::too_many_arguments)]

pub mod core;

#[cfg(feature = "swap")]
pub mod swap;

#[cfg(feature = "swap")]
mod pool;
#[cfg(feature = "swap")]
pub use pool::{Pool, PoolError};
