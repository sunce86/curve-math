//! Data-source agnostic pool construction for Yield Basis LEVAMM.
//!
//! - [`build_pool`] — converts flat [`RawYieldBasisState`] into a `YieldBasisPool`.
//!   Pure, no I/O. Requires the `build` feature (default).
//!
//! ```rust,ignore
//! use yield_basis_adapter::{RawYieldBasisState, build_pool};
//!
//! let state = RawYieldBasisState {
//!     leverage, lev_ratio, collateral_precision,
//!     fee, collateral_amount, debt, rate_mul,
//!     stored_debt, rate, rate_time,
//!     p_oracle,
//! };
//! let pool = build_pool(&state, now)?;
//! let dy = pool.get_amount_out(0, 1, dx)?;
//! ```

#[cfg(feature = "build")]
pub mod build;

#[cfg(feature = "build")]
pub use build::{build_pool, RawYieldBasisState};
