//! Adapts raw on-chain Curve pool data into [`curve_math::Pool`] instances.
//!
//! # Modules
//!
//! **Start here:**
//! - [`build`] — `build_pool(RawPoolState) → Pool`. The main entry point.
//!   Takes raw balances, decimals, amp, fees and constructs a ready-to-use Pool.
//!
//! **Don't know the variant? Detect it:**
//! - [`detect`] — `detect_variant(probing_results) → CurveVariant`. Probe a pool's
//!   on-chain functions (gamma, offpeg, MATH version, etc.) and classify it.
//!
//! **Supporting data (you need these to populate `RawPoolState`):**
//! - [`variant`] — `CurveVariant` enum: which of the 11 pool types is this?
//! - [`state`] — what on-chain fields does each variant need, and how often do they change?
//! - [`factories`] — factory contract addresses per chain, MATH→variant lookup
//! - [`discovery`] — parse factory deploy events into `PoolInfo`
//! - [`metapool`] — resolve base pool address → LP token address
//!
//! # Quick start
//!
//! ```rust
//! use curve_adapter::{CurveVariant, RawPoolState, build_pool};
//! use alloy_primitives::U256;
//!
//! let state = RawPoolState {
//!     variant: CurveVariant::StableSwapV2,
//!     balances: vec![U256::from(1_000_000_000_000_000_000_000u128),
//!                    U256::from(1_000_000_000_000u128)],
//!     token_decimals: vec![18, 6],
//!     amp: U256::from(40_000u64), // A=400 * A_PRECISION=100
//!     fee: Some(U256::from(4_000_000u64)),
//!     ..Default::default()
//! };
//!
//! let pool = build_pool(&state).unwrap();
//! let dy = pool.get_amount_out(0, 1, U256::from(1_000_000_000_000_000_000u128));
//! ```

mod detect;
mod discovery;
mod factories;
mod metapool;
mod state;
mod variant;

#[cfg(feature = "build")]
mod build;

pub use detect::{detect_variant, DetectError, ProbingResults};
pub use discovery::{
    parse_stableswap_ng_deploy, parse_tricrypto_ng_deploy, parse_twocrypto_ng_deploy, PoolInfo,
};
pub use factories::{factories, Chain, ChainConfig, DeployEvent, Factory};
pub use metapool::resolve_base_pool_lp_token;
pub use state::{state_requirements, StateRequirements, UpdateFrequency};
pub use variant::CurveVariant;

#[cfg(feature = "build")]
pub use build::{build_pool, interpolate_a, BuildError, RawPoolState};
