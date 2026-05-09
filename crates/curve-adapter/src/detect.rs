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
//! # Detection order (StableSwap)
//!
//! 1. `stored_rates()` → **StableSwapNG** (all NG pools, including v5+ crvUSD
//!    factory pools that lack `offpeg_fee_multiplier()`)
//! 2. `offpeg_fee_multiplier()` without `stored_rates()` → **StableSwapALend**
//! 3. `version()` → **StableSwapNG** (v6+ crvUSD factory pools that lack both
//!    `stored_rates()` and `offpeg_fee_multiplier()`)
//! 4. `base_pool()` → **StableSwapMeta**
//! 5. `balances(int128)` → **StableSwapV0**
//! 6. Known address → **StableSwapV0** / **StableSwapV1** / **StableSwapSTETH**
//! 7. Fallback → **StableSwapV2**
//!
//! # Limitations
//!
//! MetaPool Factory proxy pools lack `base_pool()`. Without factory context,
//! they are misclassified as `StableSwapV2`. Factory-aware detection (via
//! deploy events or `factory.is_meta()`) is more reliable. See
//! [`factories`](crate::factories) for factory→variant mapping.

use alloy_primitives::{address, Address};

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
/// has_version            ← call version() on the pool itself
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
    /// Note: v5+ crvUSD StableSwap Factory pools may lack this while still
    /// being NG (detected via `stored_rates()` instead).
    pub has_offpeg_fee_multiplier: bool,

    /// Pool has `stored_rates()` → StableSwapNG. Present on all NG pools
    /// including v5+ crvUSD factory pools that lack `offpeg_fee_multiplier()`.
    /// ALend does not have this.
    pub has_stored_rates: bool,

    /// Pool has `version()` getter → NG-era pool (v5+, v6+, v7+).
    /// All NG pools have this. Legacy (V0/V1/V2/Meta/ALend) do not.
    /// Catches v6+ crvUSD factory pools that lack both `stored_rates()`
    /// and `offpeg_fee_multiplier()`.
    pub has_version: bool,

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
            if KNOWN_TRICRYPTO_V1.contains(&probing.pool_address) {
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
    // stored_rates → NG (covers standard NG pools with offpeg_fee_multiplier
    // AND v5+ crvUSD factory pools that have stored_rates without offpeg)
    if probing.has_stored_rates {
        return CurveVariant::StableSwapNG;
    }

    // offpeg_fee_multiplier without stored_rates → ALend
    if probing.has_offpeg_fee_multiplier {
        return CurveVariant::StableSwapALend;
    }

    // version() → NG (covers v6+ crvUSD factory pools that lack both
    // stored_rates and offpeg_fee_multiplier)
    if probing.has_version {
        return CurveVariant::StableSwapNG;
    }

    // base_pool() → Meta
    if probing.has_base_pool {
        return CurveVariant::StableSwapMeta;
    }

    // balances(int128) → V0
    if probing.has_int128_balances {
        return CurveVariant::StableSwapV0;
    }

    // Fallback: known addresses for V0/V1/STETH, else V2
    let addr = probing.pool_address;
    if KNOWN_V0.contains(&addr) {
        CurveVariant::StableSwapV0
    } else if KNOWN_V1.contains(&addr) {
        CurveVariant::StableSwapV1
    } else if KNOWN_STETH.contains(&addr) {
        CurveVariant::StableSwapSTETH
    } else {
        CurveVariant::StableSwapV2
    }
}

//
// These pools cannot be reliably distinguished on-chain and require
// address-based lookup. These are COMPLETE lists — no new pools of these
// types can be created because no factory exists for them. All pre-factory
// pools are deployed manually and the set is fixed.

const KNOWN_TRICRYPTO_V1: [Address; 2] = [
    address!("D51a44d3FaE010294C616388b506AcdA1bfAAE46"), // tricrypto2 (USDT/WBTC/WETH)
    address!("80466c64868E1ab14a1Ddf27A676C3fcBE638Fe5"), // tricrypto (original)
];

const KNOWN_V0: [Address; 8] = [
    address!("A5407eAE9Ba41422680e2e00537571bcC53efBfD"), // sUSD
    address!("A2B47E3D5c44877cca798226B7B8118F9BFb7A56"), // compound
    address!("79a8C46DeA5aDa233ABaFFD40F3A0A2B1e5A4F27"), // busd
    address!("45F783CCE6B7FF23B2ab2D70e416cdb7D6055f51"), // y
    address!("52EA46506B9CC5Ef470C5bf89f17Dc28bB35D85C"), // usdt
    address!("06364f10B501e868329afBc005b3492902d6C763"), // pax
    address!("93054188d876f558f4a66B2EF1d97d16eDf0895B"), // ren
    address!("7fC77b5c7614E1533320Ea6DDc2Eb61fa00A9714"), // sbtc
];

const KNOWN_V1: [Address; 2] = [
    address!("bEbc44782C7dB0a1A60Cb6fe97d0b483032FF1C7"), // 3pool
    address!("4CA9b3063Ec5866A4B82E437059D2C43d1be596F"), // hbtc
];

const KNOWN_STETH: [Address; 1] = [
    address!("DC24316b9AE028F1497c275EB9192a3Ea0f67022"), // Lido stETH/ETH
];

/// Standard EIP-1167 minimal proxy bytecode prefix (10 bytes).
/// Followed by 20 bytes of implementation address, then [`EIP1167_SUFFIX`].
const EIP1167_PREFIX: &[u8] = b"\x36\x3d\x3d\x37\x3d\x3d\x3d\x36\x3d\x73";

/// Standard EIP-1167 minimal proxy bytecode suffix (15 bytes).
const EIP1167_SUFFIX: &[u8] = b"\x5a\xf4\x3d\x82\x80\x3e\x90\x3d\x91\x60\x2b\x57\xfd\x5b\xf3";

/// Total length of a standard EIP-1167 minimal proxy.
const EIP1167_LEN: usize = 45;

/// Returns `true` if `bytecode` is a standard EIP-1167 minimal proxy.
///
/// All three checks must pass: exact length, prefix, and suffix. This makes
/// the false positive rate effectively zero — a non-proxy contract would
/// have to coincidentally match all 25 fixed bytes at exact positions.
pub fn is_eip1167_proxy(bytecode: &[u8]) -> bool {
    bytecode.len() == EIP1167_LEN
        && bytecode.starts_with(EIP1167_PREFIX)
        && bytecode.ends_with(EIP1167_SUFFIX)
}

/// Chains supported by `curve_adapter` detection logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SupportedChain {
    /// Ethereum Mainnet (chain ID 1).
    Mainnet = 1,
    /// Base (chain ID 8453).
    Base = 8453,
    /// Arbitrum One (chain ID 42161).
    Arbitrum = 42161,
}

impl SupportedChain {
    /// Convert a numeric chain ID to a `SupportedChain`. Returns `None` for
    /// chains the adapter does not yet support.
    pub fn from_u64(id: u64) -> Option<Self> {
        match id {
            1 => Some(Self::Mainnet),
            8453 => Some(Self::Base),
            42161 => Some(Self::Arbitrum),
            _ => None,
        }
    }

    /// Wrapped native token address (WETH on EVM chains).
    pub fn wrapped_native(self) -> Address {
        match self {
            Self::Mainnet => address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"),
            Self::Base => address!("4200000000000000000000000000000000000006"),
            Self::Arbitrum => address!("82aF49447D8a07e3bd95BD0d56f35241523fBab1"),
        }
    }
}

/// Detect whether a `TwoCryptoV1` pool uses the ETH variant Newton solver.
///
/// `CurveCryptoSwap2ETH.vy` and `CurveCryptoSwap2.vy` differ in one Newton
/// step (`mul2` formula). Detection rule:
/// - Factory-deployed pools (EIP-1167 proxies pointing at the CryptoSwap
///   factory implementation) **always** use the ETH variant, regardless of
///   coin composition (e.g. CVG/crvFRAX has no WETH but uses ETH math).
/// - Legacy direct deploys: `CurveCryptoSwap2ETH.vy` was used for
///   WETH-paired pools (e.g. CRV/ETH); `CurveCryptoSwap2.vy` for the rest.
///
/// Combined check: `coins.contains(WETH) || is_eip1167_proxy(bytecode)`.
///
/// # Arguments
/// * `coins` — pool's coin addresses (read from on-chain `coins(i)` getters)
/// * `pool_bytecode` — runtime bytecode at the pool address (`eth_getCode`)
/// * `chain` — chain on which the pool is deployed
pub fn detect_eth_variant(coins: &[Address], pool_bytecode: &[u8], chain: SupportedChain) -> bool {
    coins.contains(&chain.wrapped_native()) || is_eip1167_proxy(pool_bytecode)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> Address {
        s.parse().unwrap()
    }

    #[test]
    fn detect_tricrypto_v1_by_address() {
        let probing = ProbingResults {
            has_gamma: true,
            n_coins: 3,
            has_math: false,
            math_version: None,
            has_offpeg_fee_multiplier: false,
            has_stored_rates: false,
            has_version: false,
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
            has_version: false,
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
            has_version: false,
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
            has_version: false,
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
            has_version: false,
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
            has_version: false,
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
            has_version: false,
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
            has_version: false,
            has_base_pool: false,
            has_int128_balances: false,
            pool_address: Address::ZERO,
        };
        assert!(detect_variant(&probing).is_err());
    }

    #[test]
    fn detect_stableswap_ng() {
        let probing = ProbingResults {
            has_gamma: false,
            n_coins: 2,
            has_math: false,
            math_version: None,
            has_offpeg_fee_multiplier: true,
            has_stored_rates: true,
            has_version: true,
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
    fn detect_stableswap_ng_without_offpeg() {
        // v5+ crvUSD factory pool: has stored_rates but no offpeg_fee_multiplier
        let probing = ProbingResults {
            has_gamma: false,
            n_coins: 2,
            has_math: false,
            math_version: None,
            has_offpeg_fee_multiplier: false,
            has_stored_rates: true,
            has_version: true,
            has_base_pool: false,
            has_int128_balances: false,
            pool_address: addr("0x1539c2461d7432cc114b0903f1824079BfCA2C92"),
        };
        assert_eq!(
            detect_variant(&probing).unwrap(),
            CurveVariant::StableSwapNG
        );
    }

    #[test]
    fn detect_stableswap_ng_via_version() {
        // v6.0.1 crvUSD factory pool: has version() but no stored_rates, no offpeg
        let probing = ProbingResults {
            has_gamma: false,
            n_coins: 3,
            has_math: false,
            math_version: None,
            has_offpeg_fee_multiplier: false,
            has_stored_rates: false,
            has_version: true,
            has_base_pool: false,
            has_int128_balances: false,
            pool_address: addr("0x4DEcE678ceceb27446b35C672dC7d61F30bAD69E"),
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
            has_version: false,
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
            has_version: false,
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
            has_version: false,
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
            has_version: false,
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
            has_version: false,
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
    fn detect_stableswap_steth_by_known_address() {
        let probing = ProbingResults {
            has_gamma: false,
            n_coins: 2,
            has_math: false,
            math_version: None,
            has_offpeg_fee_multiplier: false,
            has_stored_rates: false,
            has_version: false,
            has_base_pool: false,
            has_int128_balances: false,
            pool_address: addr("0xDC24316b9AE028F1497c275EB9192a3Ea0f67022"),
        };
        assert_eq!(
            detect_variant(&probing).unwrap(),
            CurveVariant::StableSwapSTETH
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
            has_version: false,
            has_base_pool: false,
            has_int128_balances: false,
            pool_address: addr("0xDcEF968d416a41Cdac0ED8702fAC8128A64241A2"),
        };
        assert_eq!(
            detect_variant(&probing).unwrap(),
            CurveVariant::StableSwapV2
        );
    }

    // ── eth_variant detection ──────────────────────────────────────────────

    fn weth_mainnet() -> Address {
        addr("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2")
    }

    /// Build a synthetic 45-byte EIP-1167 proxy with the given implementation address.
    fn synthetic_proxy(impl_addr: Address) -> Vec<u8> {
        let mut out = Vec::with_capacity(EIP1167_LEN);
        out.extend_from_slice(EIP1167_PREFIX);
        out.extend_from_slice(impl_addr.as_slice());
        out.extend_from_slice(EIP1167_SUFFIX);
        out
    }

    #[test]
    fn is_eip1167_recognizes_standard_proxy() {
        let bytecode = synthetic_proxy(addr("0xa854b3df11afe1e54ddd2d62ce03a40bd0bba100"));
        assert!(is_eip1167_proxy(&bytecode));
    }

    #[test]
    fn is_eip1167_rejects_wrong_length() {
        let mut bytecode = synthetic_proxy(addr("0xa854b3df11afe1e54ddd2d62ce03a40bd0bba100"));
        bytecode.push(0x00);
        assert!(!is_eip1167_proxy(&bytecode));
    }

    #[test]
    fn is_eip1167_rejects_corrupted_prefix() {
        let mut bytecode = synthetic_proxy(addr("0xa854b3df11afe1e54ddd2d62ce03a40bd0bba100"));
        bytecode[0] = 0xff;
        assert!(!is_eip1167_proxy(&bytecode));
    }

    #[test]
    fn is_eip1167_rejects_corrupted_suffix() {
        let mut bytecode = synthetic_proxy(addr("0xa854b3df11afe1e54ddd2d62ce03a40bd0bba100"));
        bytecode[EIP1167_LEN - 1] = 0x00;
        assert!(!is_eip1167_proxy(&bytecode));
    }

    #[test]
    fn is_eip1167_rejects_full_contract_bytecode() {
        // Realistic Vyper-compiled runtime starts with 0x60 (PUSH1) typically.
        let bytecode = vec![0x60u8; 5000];
        assert!(!is_eip1167_proxy(&bytecode));
    }

    #[test]
    fn supported_chain_from_u64_known() {
        assert_eq!(SupportedChain::from_u64(1), Some(SupportedChain::Mainnet));
        assert_eq!(SupportedChain::from_u64(8453), Some(SupportedChain::Base));
        assert_eq!(
            SupportedChain::from_u64(42161),
            Some(SupportedChain::Arbitrum)
        );
    }

    #[test]
    fn supported_chain_from_u64_unknown() {
        assert_eq!(SupportedChain::from_u64(0), None);
        assert_eq!(SupportedChain::from_u64(137), None); // Polygon: not yet supported
    }

    #[test]
    fn detect_eth_variant_weth_paired_legacy() {
        // CRV/ETH-style: direct deploy with WETH coin → ETH variant
        let coins = [
            addr("0xD533a949740bb3306d119CC777fa900bA034cd52"),
            weth_mainnet(),
        ];
        let bytecode = vec![0x60u8; 5000]; // full deploy, not proxy
        assert!(detect_eth_variant(
            &coins,
            &bytecode,
            SupportedChain::Mainnet
        ));
    }

    #[test]
    fn detect_eth_variant_factory_no_weth() {
        // CVG/crvFRAX-style: factory pool, no WETH → still ETH variant
        let coins = [
            addr("0x97efFB790f2fbB701D88f89DB4521348A2B77be8"),
            addr("0x3175Df0976dFA876431C2E9eE6Bc45b65d3473CC"),
        ];
        let bytecode = synthetic_proxy(addr("0xa854b3df11afe1e54ddd2d62ce03a40bd0bba100"));
        assert!(detect_eth_variant(
            &coins,
            &bytecode,
            SupportedChain::Mainnet
        ));
    }

    #[test]
    fn detect_eth_variant_legacy_direct_no_weth() {
        // EURC/3Crv-style: legacy direct deploy, no WETH → non-ETH variant
        let coins = [
            addr("0x1aBaEA1f7C830bD89Acc67eC4af516284b1bC33c"),
            addr("0x6c3F90f043a72FA612cbac8115EE7e52BDe6E490"),
        ];
        let bytecode = vec![0x60u8; 5000]; // full deploy, not proxy
        assert!(!detect_eth_variant(
            &coins,
            &bytecode,
            SupportedChain::Mainnet
        ));
    }
}
