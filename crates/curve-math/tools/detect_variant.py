#!/usr/bin/env python3
"""
Detect Curve pool variant by probing on-chain functions.

Usage:
    python tools/detect_variant.py <pool_address> [--rpc <url>]
    python tools/detect_variant.py 0xbEbc44782C7dB0a1A60Cb6fe97d0b483032FF1C7

Known Ethereum mainnet factories (all use pool_count/pool_list interface):
  NG factories:
    0x6A8cbed756804B16E05E741eDaBd5cB544AE21bf  StableSwap-NG       → StableSwapNG
    0x98ee851a00abee0d95d08cf4ca2bdce32aeaaf7f  TwoCrypto-NG        → TwoCryptoNG / TwoCryptoStable
    0x0c0e5f2ff0ff18a3be9b835635039256dc4b4963  TriCrypto-NG        → TriCryptoNG
  Legacy factories:
    0xB9fC157394Af804a3578134A6585C0dc9cc990d4  MetaPool Factory    → StableSwapMeta
    0xF18056Bbd320E96A48e3Fbf8bC061322531aac99  CryptoSwap Factory  → TwoCryptoV1
    0x4F8846Ae9380B90d2E71D5e3D042dff3E7ebb40d  crvUSD StableSwap   → auto-detect
"""

import json
import os
import sys

try:
    from web3 import Web3
except ImportError:
    sys.exit("pip install web3")


def probe(w3, addr, abi_json, fn_name, args=None, block="latest"):
    """Try calling a function. Returns (True, result) or (False, None)."""
    try:
        c = w3.eth.contract(address=Web3.to_checksum_address(addr), abi=json.loads(abi_json))
        fn = getattr(c.functions, fn_name)(*(args or []))
        result = fn.call(block_identifier=block)
        return True, result
    except Exception:
        return False, None


def detect_variant(w3, addr, block="latest"):
    """Detect pool variant by probing on-chain functions."""
    addr = Web3.to_checksum_address(addr)

    # 1. Has gamma()? → CryptoSwap
    has_gamma, gamma = probe(w3, addr,
        '[{"name":"gamma","outputs":[{"type":"uint256"}],"inputs":[],"stateMutability":"view","type":"function"}]',
        "gamma", block=block)

    if has_gamma:
        # Count coins
        n_coins = 0
        for i in range(4):
            ok, _ = probe(w3, addr,
                '[{"name":"coins","outputs":[{"type":"address"}],"inputs":[{"type":"uint256"}],"stateMutability":"view","type":"function"}]',
                "coins", [i], block=block)
            if ok:
                n_coins += 1
            else:
                break

        if n_coins == 3:
            # TriCryptoV1 vs TriCryptoNG — cannot be reliably distinguished on-chain
            # (both have price_scale(uint256)). Use known addresses.
            KNOWN_TRICRYPTO_V1 = {
                "0xd51a44d3fae010294c616388b506acda1bfaae46",  # tricrypto2 (USDT/WBTC/WETH)
                "0x80466c64868e1ab14a1ddf27a676c3fcbe638fe5",  # tricrypto (original)
            }
            if addr.lower() in KNOWN_TRICRYPTO_V1:
                return "TriCryptoV1"
            return "TriCryptoNG"

        if n_coins == 2:
            # TwoCrypto — check MATH version
            has_math, math_addr = probe(w3, addr,
                '[{"name":"MATH","outputs":[{"type":"address"}],"inputs":[],"stateMutability":"view","type":"function"}]',
                "MATH", block=block)
            if has_math and math_addr:
                _, version = probe(w3, math_addr,
                    '[{"name":"version","outputs":[{"type":"string"}],"inputs":[],"stateMutability":"view","type":"function"}]',
                    "version", block=block)
                if version in ("v2.0.0", "v2.1.0"):
                    return "TwoCryptoNG"
                elif version == "v0.1.0":
                    return "TwoCryptoStable"
                else:
                    return f"TwoCryptoNG (unknown MATH {version})"
            else:
                # Legacy TwoCrypto (no MATH() function — inline math)
                return "TwoCryptoV1"

        return f"CryptoSwap ({n_coins}-coin, unknown variant)"

    # 2. StableSwap — check offpeg_fee_multiplier
    has_offpeg, _ = probe(w3, addr,
        '[{"name":"offpeg_fee_multiplier","outputs":[{"type":"uint256"}],"inputs":[],"stateMutability":"view","type":"function"}]',
        "offpeg_fee_multiplier", block=block)

    if has_offpeg:
        # ALend or NG — check for stored_rates (NG has it)
        has_stored_rates, _ = probe(w3, addr,
            '[{"name":"stored_rates","outputs":[{"type":"uint256[]"}],"inputs":[],"stateMutability":"view","type":"function"}]',
            "stored_rates", block=block)
        if has_stored_rates:
            return "StableSwapNG"
        else:
            return "StableSwapALend"

    # 3. Has base_pool()? → Meta
    has_base_pool, _ = probe(w3, addr,
        '[{"name":"base_pool","outputs":[{"type":"address"}],"inputs":[],"stateMutability":"view","type":"function"}]',
        "base_pool", block=block)
    if has_base_pool:
        return "StableSwapMeta"

    # 4. balances(int128) works? → V0
    has_int128, _ = probe(w3, addr,
        '[{"name":"balances","outputs":[{"type":"uint256"}],"inputs":[{"type":"int128"}],"stateMutability":"view","type":"function"}]',
        "balances", [0], block=block)
    if has_int128:
        return "StableSwapV0"

    # 5. V0 vs V1 vs V2 cannot be reliably distinguished on-chain.
    # Use known addresses from curve-contract/contracts/pools/pooldata.json:
    # - CurveTokenV1 pools → V0 (sUSD, compound, busd, y, usdt, pax, ren, sbtc)
    # - CurveTokenV2 plain pools → V1 (3pool, hbtc)
    # - CurveTokenV3 plain pools without base_pool → V2 (steth, seth, reth, link, saave, aeth)
    KNOWN_V0 = {
        "0xa5407eae9ba41422680e2e00537571bcc53efbfd",  # sUSD
        "0xa2b47e3d5c44877cca798226b7b8118f9bfb7a56",  # compound
        "0x79a8c46dea5ada233abaffd40f3a0a2b1e5a4f27",  # busd
        "0x45f783cce6b7ff23b2ab2d70e416cdb7d6055f51",  # y
        "0x52ea46506b9cc5ef470c5bf89f17dc28bb35d85c",  # usdt
        "0x06364f10b501e868329afbc005b3492902d6c763",  # pax
        "0x93054188d876f558f4a66b2ef1d97d16edf0895b",  # ren
        "0x7fc77b5c7614e1533320ea6ddc2eb61fa00a9714",  # sbtc
    }
    KNOWN_V1 = {
        "0xbebc44782c7db0a1a60cb6fe97d0b483032ff1c7",  # 3pool
        "0x4ca9b3063ec5866a4b82e437059d2c43d1be596f",  # hbtc
    }
    addr_lower = addr.lower()
    if addr_lower in KNOWN_V0:
        return "StableSwapV0"
    if addr_lower in KNOWN_V1:
        return "StableSwapV1"

    # Default: V2 for unknown plain StableSwap pools
    return "StableSwapV2"


def main():
    import argparse
    parser = argparse.ArgumentParser(description="Detect Curve pool variant")
    parser.add_argument("address", help="Pool contract address")
    parser.add_argument("--rpc", help="RPC URL")
    parser.add_argument("--chain-id", type=int, default=1)
    args = parser.parse_args()

    rpc = args.rpc or os.environ.get(f"RPC_URL_{args.chain_id}") or os.environ.get("RPC_URL")
    if not rpc:
        sys.exit(f"RPC_URL_{args.chain_id} or --rpc required")

    w3 = Web3(Web3.HTTPProvider(rpc))
    block = w3.eth.block_number - 5
    variant = detect_variant(w3, args.address, block)
    print(f"{args.address} → {variant}")


if __name__ == "__main__":
    main()
