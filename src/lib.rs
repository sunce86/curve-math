#![allow(clippy::too_many_arguments)]

pub mod core;

#[cfg(feature = "swap")]
pub mod swap;

#[cfg(feature = "swap")]
mod pool;
#[cfg(feature = "swap")]
pub use pool::Pool;
