//! Detect Curve pool variant from on-chain probing results.
//!
//! This module ports the logic from `detect_variant.py` into Rust, but
//! **without any RPC calls**. The consumer probes the pool contract and
//! reports results via [`ProbingResults`], then [`detect_variant`] returns
//! the appropriate [`CurveVariant`].
//!
//! # When to use this
//!
//! Use this module when you have a pool address and need to identify
//! its variant via RPC probing (e.g., a standalone indexer discovering
//! unknown pools). If you already know the variant from another source
//! (deploy events, protocol metadata, etc.), skip this and pass the
//! variant directly to [`RawPoolState`](crate::RawPoolState).
//!
//! # Limitations
//!
//! MetaPool Factory proxy pools lack `base_pool()`. Without factory context,
//! they are misclassified as `StableSwapV2`. Factory-aware detection (via
//! deploy events or `factory.is_meta()`) is more reliable. See
//! [`factories`](crate::factories) for factory→variant mapping.

use alloy_primitives::Address;

use crate::CurveVariant;

/// Results of on-chain function probing.
///
/// The consumer calls these getters on the pool contract and reports
/// whether each call succeeded. No actual values are needed (except
/// `math_version`) — only success/failure matters.
///
/// # How to populate
///
/// For each field, try calling the corresponding on-chain function.
/// If the call reverts, set the field to `false` / `None`.
///
/// ```text
/// has_gamma             ← call gamma()
/// n_coins               ← count how many coins(i) calls succeed (i = 0, 1, 2, ...)
/// has_math              ← call MATH() → returns address
/// math_version          ← call version() on the MATH address
/// has_offpeg_fee_multiplier ← call offpeg_fee_multiplier()
/// has_stored_rates       ← call stored_rates()
/// has_base_pool          ← call base_pool()
/// has_int128_balances    ← call balances(int128(0))
/// ```
pub struct ProbingResults {
    /// Pool has `gamma()` getter → CryptoSwap variant.
    pub has_gamma: bool,

    /// Number of coins in the pool (count `coins(i)` calls that succeed).
    pub n_coins: usize,

    /// Pool has `MATH()` getter → TwoCrypto-NG with external math contract.
    pub has_math: bool,

    /// Version string from `MATH().version()`. E.g. `"v2.0.0"`, `"v2.1.0"`, `"v0.1.0"`.
    pub math_version: Option<String>,

    /// Pool has `offpeg_fee_multiplier()` → StableSwapNG or StableSwapALend.
    pub has_offpeg_fee_multiplier: bool,

    /// Pool has `stored_rates()` → StableSwapNG (ALend does not have this).
    pub has_stored_rates: bool,

    /// Pool has `base_pool()` → StableSwapMeta.
    /// Note: MetaPool Factory proxy pools lack this getter.
    pub has_base_pool: bool,

    /// `balances(int128(0))` call succeeds → V0-era pool (oldest interface).
    pub has_int128_balances: bool,

    /// Pool contract address (used for known-address fallback for V0/V1/TriCryptoV1).
    pub pool_address: Address,
}

/// Error returned when variant cannot be determined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectError {
    pub message: String,
}

impl std::fmt::Display for DetectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cannot detect variant: {}", self.message)
    }
}

impl std::error::Error for DetectError {}

/// Detect pool variant from probing results.
///
/// This is a pure function — no RPC calls. The consumer probes the pool
/// and passes results.
///
/// # Errors
///
/// Returns `DetectError` if the pool has `gamma()` but an unsupported
/// coin count (not 2 or 3).
pub fn detect_variant(probing: &ProbingResults) -> Result<CurveVariant, DetectError> {
    if probing.has_gamma {
        detect_cryptoswap(probing)
    } else {
        Ok(detect_stableswap(probing))
    }
}

fn detect_cryptoswap(probing: &ProbingResults) -> Result<CurveVariant, DetectError> {
    match probing.n_coins {
        3 => {
            if is_known_tricrypto_v1(probing.pool_address) {
                Ok(CurveVariant::TriCryptoV1)
            } else {
                Ok(CurveVariant::TriCryptoNG)
            }
        }
        2 => {
            if probing.has_math {
                match probing.math_version.as_deref() {
                    Some("v2.0.0" | "v2.1.0") => Ok(CurveVariant::TwoCryptoNG),
                    Some("v0.1.0") => Ok(CurveVariant::TwoCryptoStable),
                    // Unknown MATH version — default to TwoCryptoNG
                    _ => Ok(CurveVariant::TwoCryptoNG),
                }
            } else {
                // No MATH() function → legacy TwoCrypto with inline math
                Ok(CurveVariant::TwoCryptoV1)
            }
        }
        n => Err(DetectError {
            message: format!("CryptoSwap pool with {n} coins — expected 2 or 3"),
        }),
    }
}

fn detect_stableswap(probing: &ProbingResults) -> CurveVariant {
    // offpeg_fee_multiplier → NG or ALend
    if probing.has_offpeg_fee_multiplier {
        if probing.has_stored_rates {
            return CurveVariant::StableSwapNG;
        } else {
            return CurveVariant::StableSwapALend;
        }
    }

    // base_pool() → Meta
    if probing.has_base_pool {
        return CurveVariant::StableSwapMeta;
    }

    // balances(int128) → V0
    if probing.has_int128_balances {
        return CurveVariant::StableSwapV0;
    }

    // Fallback: known addresses for V0/V1, else V2
    let addr_lower = format!("{:?}", probing.pool_address).to_lowercase();
    if is_known_v0(&addr_lower) {
        CurveVariant::StableSwapV0
    } else if is_known_v1(&addr_lower) {
        CurveVariant::StableSwapV1
    } else {
        CurveVariant::StableSwapV2
    }
}

// ── Known address sets ───────────────────────────────────────────────────────
//
// These pools cannot be reliably distinguished on-chain and require
// address-based lookup. These are COMPLETE lists — no new pools of these
// types can be created because no factory exists for them. All pre-factory
// pools are deployed manually and the set is fixed.
//
// Verified against legacy_ethereum.toml: V0=8, V1=2, TriCryptoV1=2.

fn is_known_tricrypto_v1(addr: Address) -> bool {
    let addr_lower = format!("{:?}", addr).to_lowercase();
    matches!(
        addr_lower.as_str(),
        // tricrypto2 (USDT/WBTC/WETH)
        "0xd51a44d3fae010294c616388b506acda1bfaae46"
        // tricrypto (original)
        | "0x80466c64868e1ab14a1ddf27a676c3fcbe638fe5"
    )
}

fn is_known_v0(addr_lower: &str) -> bool {
    matches!(
        addr_lower,
        "0xa5407eae9ba41422680e2e00537571bcc53efbfd"  // sUSD
        | "0xa2b47e3d5c44877cca798226b7b8118f9bfb7a56" // compound
        | "0x79a8c46dea5ada233abaffd40f3a0a2b1e5a4f27" // busd
        | "0x45f783cce6b7ff23b2ab2d70e416cdb7d6055f51" // y
        | "0x52ea46506b9cc5ef470c5bf89f17dc28bb35d85c" // usdt
        | "0x06364f10b501e868329afbc005b3492902d6c763" // pax
        | "0x93054188d876f558f4a66b2ef1d97d16edf0895b" // ren
        | "0x7fc77b5c7614e1533320ea6ddc2eb61fa00a9714" // sbtc
    )
}

fn is_known_v1(addr_lower: &str) -> bool {
    matches!(
        addr_lower,
        "0xbebc44782c7db0a1a60cb6fe97d0b483032ff1c7"  // 3pool
        | "0x4ca9b3063ec5866a4b82e437059d2c43d1be596f" // hbtc
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> Address {
        s.parse().unwrap()
    }

    // ── CryptoSwap detection ─────────────────────────────────────────────

    #[test]
    fn detect_tricrypto_v1_by_address() {
        let probing = ProbingResults {
            has_gamma: true,
            n_coins: 3,
            has_math: false,
            math_version: None,
            has_offpeg_fee_multiplier: false,
            has_stored_rates: false,
            has_base_pool: false,
            has_int128_balances: false,
            pool_address: addr("0xD51a44d3FaE010294C616388b506AcdA1bfAAE46"),
        };
        assert_eq!(detect_variant(&probing).unwrap(), CurveVariant::TriCryptoV1);
    }

    #[test]
    fn detect_tricrypto_ng_unknown_3coin() {
        let probing = ProbingResults {
            has_gamma: true,
            n_coins: 3,
            has_math: false,
            math_version: None,
            has_offpeg_fee_multiplier: false,
            has_stored_rates: false,
            has_base_pool: false,
            has_int128_balances: false,
            pool_address: addr("0x7F86Bf177Dd4F3494b841a37e810A34dD56c829B"),
        };
        assert_eq!(detect_variant(&probing).unwrap(), CurveVariant::TriCryptoNG);
    }

    #[test]
    fn detect_twocrypto_ng_v200() {
        let probing = ProbingResults {
            has_gamma: true,
            n_coins: 2,
            has_math: true,
            math_version: Some("v2.0.0".to_string()),
            has_offpeg_fee_multiplier: false,
            has_stored_rates: false,
            has_base_pool: false,
            has_int128_balances: false,
            pool_address: addr("0xfb8b95Fb2296a0Ad4b6b1419fdAA5AA5F13e4009"),
        };
        assert_eq!(detect_variant(&probing).unwrap(), CurveVariant::TwoCryptoNG);
    }

    #[test]
    fn detect_twocrypto_ng_v210() {
        let probing = ProbingResults {
            has_gamma: true,
            n_coins: 2,
            has_math: true,
            math_version: Some("v2.1.0".to_string()),
            has_offpeg_fee_multiplier: false,
            has_stored_rates: false,
            has_base_pool: false,
            has_int128_balances: false,
            pool_address: Address::ZERO,
        };
        assert_eq!(detect_variant(&probing).unwrap(), CurveVariant::TwoCryptoNG);
    }

    #[test]
    fn detect_twocrypto_stable_v010() {
        let probing = ProbingResults {
            has_gamma: true,
            n_coins: 2,
            has_math: true,
            math_version: Some("v0.1.0".to_string()),
            has_offpeg_fee_multiplier: false,
            has_stored_rates: false,
            has_base_pool: false,
            has_int128_balances: false,
            pool_address: addr("0x6e5492F8ea2370844EE098A56DD88e1717e4A9C2"),
        };
        assert_eq!(
            detect_variant(&probing).unwrap(),
            CurveVariant::TwoCryptoStable
        );
    }

    #[test]
    fn detect_twocrypto_v1_no_math() {
        let probing = ProbingResults {
            has_gamma: true,
            n_coins: 2,
            has_math: false,
            math_version: None,
            has_offpeg_fee_multiplier: false,
            has_stored_rates: false,
            has_base_pool: false,
            has_int128_balances: false,
            pool_address: addr("0x8301AE4fc9c624d1D396cbDAa1ed877821D7C511"),
        };
        assert_eq!(detect_variant(&probing).unwrap(), CurveVariant::TwoCryptoV1);
    }

    #[test]
    fn detect_twocrypto_ng_unknown_math_version() {
        let probing = ProbingResults {
            has_gamma: true,
            n_coins: 2,
            has_math: true,
            math_version: Some("v3.0.0".to_string()),
            has_offpeg_fee_multiplier: false,
            has_stored_rates: false,
            has_base_pool: false,
            has_int128_balances: false,
            pool_address: Address::ZERO,
        };
        // Unknown version defaults to TwoCryptoNG
        assert_eq!(detect_variant(&probing).unwrap(), CurveVariant::TwoCryptoNG);
    }

    #[test]
    fn detect_crypto_unsupported_coin_count() {
        let probing = ProbingResults {
            has_gamma: true,
            n_coins: 4,
            has_math: false,
            math_version: None,
            has_offpeg_fee_multiplier: false,
            has_stored_rates: false,
            has_base_pool: false,
            has_int128_balances: false,
            pool_address: Address::ZERO,
        };
        assert!(detect_variant(&probing).is_err());
    }

    // ── StableSwap detection ─────────────────────────────────────────────

    #[test]
    fn detect_stableswap_ng() {
        let probing = ProbingResults {
            has_gamma: false,
            n_coins: 2,
            has_math: false,
            math_version: None,
            has_offpeg_fee_multiplier: true,
            has_stored_rates: true,
            has_base_pool: false,
            has_int128_balances: false,
            pool_address: addr("0xF36a4BA50C603204c3FC6d2dA8b78A7b69CBC67d"),
        };
        assert_eq!(
            detect_variant(&probing).unwrap(),
            CurveVariant::StableSwapNG
        );
    }

    #[test]
    fn detect_stableswap_alend() {
        let probing = ProbingResults {
            has_gamma: false,
            n_coins: 3,
            has_math: false,
            math_version: None,
            has_offpeg_fee_multiplier: true,
            has_stored_rates: false,
            has_base_pool: false,
            has_int128_balances: false,
            pool_address: addr("0xDeBF20617708857ebe4F679508E7b7863a8A8EeE"),
        };
        assert_eq!(
            detect_variant(&probing).unwrap(),
            CurveVariant::StableSwapALend
        );
    }

    #[test]
    fn detect_stableswap_meta() {
        let probing = ProbingResults {
            has_gamma: false,
            n_coins: 2,
            has_math: false,
            math_version: None,
            has_offpeg_fee_multiplier: false,
            has_stored_rates: false,
            has_base_pool: true,
            has_int128_balances: false,
            pool_address: addr("0x4f062658EaAF2C1ccf8C8e36D6824CDf41167956"),
        };
        assert_eq!(
            detect_variant(&probing).unwrap(),
            CurveVariant::StableSwapMeta
        );
    }

    #[test]
    fn detect_stableswap_v0_by_int128() {
        let probing = ProbingResults {
            has_gamma: false,
            n_coins: 4,
            has_math: false,
            math_version: None,
            has_offpeg_fee_multiplier: false,
            has_stored_rates: false,
            has_base_pool: false,
            has_int128_balances: true,
            pool_address: addr("0xA5407eAE9Ba41422680e2e00537571bcC53efBfD"),
        };
        assert_eq!(
            detect_variant(&probing).unwrap(),
            CurveVariant::StableSwapV0
        );
    }

    #[test]
    fn detect_stableswap_v0_by_known_address() {
        // Even without int128 probe, known address identifies V0
        let probing = ProbingResults {
            has_gamma: false,
            n_coins: 4,
            has_math: false,
            math_version: None,
            has_offpeg_fee_multiplier: false,
            has_stored_rates: false,
            has_base_pool: false,
            has_int128_balances: false,
            pool_address: addr("0xA5407eAE9Ba41422680e2e00537571bcC53efBfD"),
        };
        assert_eq!(
            detect_variant(&probing).unwrap(),
            CurveVariant::StableSwapV0
        );
    }

    #[test]
    fn detect_stableswap_v1_3pool() {
        let probing = ProbingResults {
            has_gamma: false,
            n_coins: 3,
            has_math: false,
            math_version: None,
            has_offpeg_fee_multiplier: false,
            has_stored_rates: false,
            has_base_pool: false,
            has_int128_balances: false,
            pool_address: addr("0xbEbc44782C7dB0a1A60Cb6fe97d0b483032FF1C7"),
        };
        assert_eq!(
            detect_variant(&probing).unwrap(),
            CurveVariant::StableSwapV1
        );
    }

    #[test]
    fn detect_stableswap_v2_default() {
        let probing = ProbingResults {
            has_gamma: false,
            n_coins: 2,
            has_math: false,
            math_version: None,
            has_offpeg_fee_multiplier: false,
            has_stored_rates: false,
            has_base_pool: false,
            has_int128_balances: false,
            pool_address: addr("0xDcEF968d416a41Cdac0ED8702fAC8128A64241A2"),
        };
        assert_eq!(
            detect_variant(&probing).unwrap(),
            CurveVariant::StableSwapV2
        );
    }
}
