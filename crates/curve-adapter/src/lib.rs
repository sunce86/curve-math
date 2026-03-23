//! Curve Finance pool discovery, variant detection, state requirements, and pool construction.
//!
//! This crate knows **what** Curve pools exist, **how** to identify them, and **how** to
//! construct `curve_math::Pool` instances from raw on-chain state for swap computation.
//!
//! # What this crate provides
//!
//! - **Variant detection** — classify any Curve pool into one of 11 variants
//! - **Factory event parsing** — extract pool info from NG factory deployment events
//! - **Legacy pool registry** — complete static list of pre-factory pools per chain
//! - **State requirements** — which on-chain parameters each variant needs and how often they change
//! - **Factory addresses** — per-chain factory contract addresses and MATH lookup tables
//! - **Pool construction** (`build` feature) — convert raw state into `curve_math::Pool`
//!
//! # Example
//!
//! ```rust
//! use curve_adapter::{CurveVariant, Chain, legacy_pools, factories, state_requirements};
//!
//! // Get all legacy pools on Ethereum
//! let legacy = legacy_pools(Chain::Ethereum);
//!
//! // Get factory addresses for Ethereum
//! let chain_factories = factories(Chain::Ethereum);
//!
//! // What state does a TwoCryptoNG pool need?
//! let reqs = state_requirements(CurveVariant::TwoCryptoNG);
//! ```

mod variant;
mod factories;
mod discovery;
mod legacy;
mod metapool;
mod state;

#[cfg(feature = "build")]
mod build;

pub use variant::CurveVariant;
pub use factories::{Chain, ChainConfig, DeployEvent, Factory, factories};
pub use discovery::{PoolInfo, parse_stableswap_ng_deploy, parse_twocrypto_ng_deploy, parse_tricrypto_ng_deploy};
pub use legacy::{LegacyPool, legacy_pools};
pub use metapool::resolve_base_pool_lp_token;
pub use state::{StateRequirements, UpdateFrequency, state_requirements};

#[cfg(feature = "build")]
pub use build::{BuildError, RawPoolState, build_pool, interpolate_a};
