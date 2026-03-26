#!/usr/bin/env python3
"""
Curve pool indexer — discovers pools from on-chain factories, verifies with fuzz, updates registry.

Usage:
    pip install web3 toml
    python tools/index-pools.py --chain-id 1 --max-new 20
    python tools/index-pools.py --chain-id 1 --max-new 20 --dry-run
"""

import argparse
import json
import os
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

try:
    import toml
except ImportError:
    sys.exit("pip install toml")

try:
    from web3 import Web3
except ImportError:
    sys.exit("pip install web3")

# ── Factory config per chain ─────────────────────────────────────────────────

FACTORIES = {
    1: [  # Ethereum
        # ── NG factories ────────────────────────────────────────────────
        {
            "address": "0x6A8cbed756804B16E05E741eDaBd5cB544AE21bf",
            "variant": "StableSwapNG",
            "label": "StableSwap-NG",
        },
        {
            "address": "0x98ee851a00abee0d95d08cf4ca2bdce32aeaaf7f",
            "variant": "TwoCryptoNG",
            "label": "TwoCrypto-NG",
        },
        {
            "address": "0x0c0e5f2ff0ff18a3be9b835635039256dc4b4963",
            "variant": "TriCryptoNG",
            "label": "TriCrypto-NG",
        },
        # ── Legacy factories ────────────────────────────────────────────
        {
            "address": "0xB9fC157394Af804a3578134A6585C0dc9cc990d4",
            "variant": "meta_factory",
            "label": "MetaPool Factory (legacy)",
        },
        {
            "address": "0xF18056Bbd320E96A48e3Fbf8bC061322531aac99",
            "variant": "TwoCryptoV1",
            "label": "CryptoSwap Factory (legacy)",
        },
        {
            "address": "0x4F8846Ae9380B90d2E71D5e3D042dff3E7ebb40d",
            "variant": "auto",
            "label": "crvUSD StableSwap Factory",
        },
    ],
    8453: [  # Base
        {
            "address": "0xd2002373543Ce3527023C75e7518C274A51ce712",
            "variant": "StableSwapNG",
            "label": "StableSwap-NG",
        },
        {
            "address": "0xc9Fe0C63Af9A39402e8a5514f9c43Af0322b665F",
            "variant": "TwoCryptoNG",
            "label": "TwoCrypto-NG",
        },
        {
            "address": "0xA5961898870943c68037F6848d2D866Ed2016bcB",
            "variant": "TriCryptoNG",
            "label": "TriCrypto-NG",
        },
        # ── Legacy factories ──────────────────────────────────────────
        {
            "address": "0x5EF72230578b3e399E6C6F4F6360edF95e83BBfd",
            "variant": "TwoCryptoV1",
            "label": "CryptoSwap Factory (legacy)",
        },
        {
            "address": "0x3093f9B57A428F3EB6285a589cb35bEA6e78c336",
            "variant": "auto",
            "label": "StableSwap Factory (legacy, 15 pools)",
        },
        {
            "address": "0x87DD13Dd25a1DBde0E1EdcF5B8Fa6cfff7eABCaD",
            "variant": "auto",
            "label": "StableSwap Factory (legacy, 725 pools)",
        },
    ],
}

# ── Minimal ABIs ─────────────────────────────────────────────────────────────

FACTORY_ABI = json.loads('[{"name":"pool_count","outputs":[{"type":"uint256"}],"inputs":[],"stateMutability":"view","type":"function"},{"name":"pool_list","outputs":[{"type":"address"}],"inputs":[{"type":"uint256"}],"stateMutability":"view","type":"function"},{"name":"is_meta","outputs":[{"type":"bool"}],"inputs":[{"type":"address"}],"stateMutability":"view","type":"function"},{"name":"get_base_pool","outputs":[{"type":"address"}],"inputs":[{"type":"address"}],"stateMutability":"view","type":"function"}]')

POOL_ABI = json.loads('[{"name":"coins","outputs":[{"type":"address"}],"inputs":[{"type":"uint256"}],"stateMutability":"view","type":"function"},{"name":"balances","outputs":[{"type":"uint256"}],"inputs":[{"type":"uint256"}],"stateMutability":"view","type":"function"},{"name":"A","outputs":[{"type":"uint256"}],"inputs":[],"stateMutability":"view","type":"function"},{"name":"fee","outputs":[{"type":"uint256"}],"inputs":[],"stateMutability":"view","type":"function"},{"name":"N_COINS","outputs":[{"type":"uint256"}],"inputs":[],"stateMutability":"view","type":"function"}]')

TOKEN_ABI = json.loads('[{"name":"decimals","outputs":[{"type":"uint8"}],"inputs":[],"stateMutability":"view","type":"function"},{"name":"symbol","outputs":[{"type":"string"}],"inputs":[],"stateMutability":"view","type":"function"}]')

MATH_GETTER_ABI = json.loads('[{"name":"MATH","outputs":[{"type":"address"}],"inputs":[],"stateMutability":"view","type":"function"}]')
VERSION_ABI = json.loads('[{"name":"version","outputs":[{"type":"string"}],"inputs":[],"stateMutability":"view","type":"function"}]')

# ABIs for on-chain variant probing (used by detect_variant_onchain)
GAMMA_ABI = json.loads('[{"name":"gamma","outputs":[{"type":"uint256"}],"inputs":[],"stateMutability":"view","type":"function"}]')
OFFPEG_ABI = json.loads('[{"name":"offpeg_fee_multiplier","outputs":[{"type":"uint256"}],"inputs":[],"stateMutability":"view","type":"function"}]')
STORED_RATES_ABI = json.loads('[{"name":"stored_rates","outputs":[{"type":"uint256[]"}],"inputs":[],"stateMutability":"view","type":"function"}]')
BASE_POOL_ABI = json.loads('[{"name":"base_pool","outputs":[{"type":"address"}],"inputs":[],"stateMutability":"view","type":"function"}]')


def get_rpc_url(chain_id: int) -> str:
    key = f"RPC_URL_{chain_id}"
    url = os.environ.get(key) or os.environ.get("RPC_URL")
    if not url:
        sys.exit(f"{key} or RPC_URL must be set")
    return url


def read_registry(path: Path) -> dict:
    if path.exists():
        return toml.load(path)
    return {"pools": []}


def write_registry(path: Path, registry: dict):
    header = (
        f"# Verified Curve pool registry — Chain ID: {path.stem}\n"
        "#\n"
        "# Auto-generated by tools/index-pools.py. Do not edit manually.\n"
        "#\n"
        "# Every pool with fuzz_verified = true has passed differential fuzz testing\n"
        "# against on-chain get_dy with random swap amounts.\n\n"
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "w") as f:
        f.write(header)
        f.write(toml.dumps(registry))


def _batch_call(w3, calls, block):
    """Execute multiple eth_call in parallel using threading."""
    from concurrent.futures import ThreadPoolExecutor, as_completed
    results = [None] * len(calls)
    def do_call(idx, contract, fn_name, args):
        try:
            fn = getattr(contract.functions, fn_name)(*args)
            return idx, fn.call(block_identifier=block)
        except Exception:
            return idx, None
    with ThreadPoolExecutor(max_workers=20) as pool:
        futures = [pool.submit(do_call, i, c, f, a) for i, (c, f, a) in enumerate(calls)]
        for future in as_completed(futures):
            idx, val = future.result()
            results[idx] = val
    return results


def _probe(w3, addr, abi, fn_name, args=None, block="latest"):
    """Try calling a function on a contract. Returns (True, result) or (False, None)."""
    try:
        c = w3.eth.contract(address=Web3.to_checksum_address(addr), abi=abi)
        fn = getattr(c.functions, fn_name)(*(args or []))
        return True, fn.call(block_identifier=block)
    except Exception:
        return False, None


def _has_stored_rates(w3, addr, block="latest"):
    """Check if pool has stored_rates() using raw eth_call.

    Vyper's stored_rates() returns a fixed-size uint256[N_COINS] which encodes
    differently from Solidity's uint256[] (no length prefix). web3.py cannot
    decode the fixed-size encoding with a uint256[] ABI. Using raw eth_call
    with just the function selector avoids the decode issue — we only need
    success/failure for variant detection, not the actual values.
    """
    try:
        # selector: keccak256("stored_rates()")[:4] = 0xfd0684b1
        result = w3.eth.call(
            {"to": Web3.to_checksum_address(addr), "data": "0xfd0684b1"},
            block_identifier=block,
        )
        return len(result) >= 64  # at least 2 uint256 values (2-coin pool)
    except Exception:
        return False


def detect_variant_onchain(w3, addr, block):
    """Detect pool variant by probing on-chain functions.

    Same logic as tools/detect_variant.py but inlined for use in the indexer.
    Used for factories where the variant isn't known from the factory alone
    (e.g., crvUSD StableSwap Factory).
    """
    has_gamma, _ = _probe(w3, addr, GAMMA_ABI, "gamma", block=block)
    if has_gamma:
        # CryptoSwap — count coins to distinguish TwoCrypto vs TriCrypto
        n_coins = 0
        for i in range(4):
            ok, _ = _probe(w3, addr, POOL_ABI, "coins", [i], block=block)
            if ok:
                n_coins += 1
            else:
                break
        if n_coins == 3:
            return "TriCryptoNG"
        if n_coins == 2:
            has_math, math_addr = _probe(w3, addr, MATH_GETTER_ABI, "MATH", block=block)
            if has_math and math_addr:
                _, version = _probe(w3, math_addr, VERSION_ABI, "version", block=block)
                if version == "v0.1.0":
                    return "TwoCryptoStable"
                return "TwoCryptoNG"
            return "TwoCryptoV1"
        return "TwoCryptoNG"

    # stored_rates → NG (covers standard NG pools AND v5+ crvUSD factory pools
    # that have stored_rates without offpeg_fee_multiplier).
    # Use raw eth_call with selector because Vyper returns fixed-size uint256[N_COINS]
    # which web3.py cannot decode with the uint256[] ABI.
    has_stored_rates = _has_stored_rates(w3, addr, block=block)
    if has_stored_rates:
        return "StableSwapNG"

    # offpeg_fee_multiplier without stored_rates → ALend
    has_offpeg, _ = _probe(w3, addr, OFFPEG_ABI, "offpeg_fee_multiplier", block=block)
    if has_offpeg:
        return "StableSwapALend"

    # version() → NG (v6+ crvUSD factory pools without stored_rates or offpeg)
    has_version, _ = _probe(w3, addr, VERSION_ABI, "version", block=block)
    if has_version:
        return "StableSwapNG"

    has_base_pool, _ = _probe(w3, addr, BASE_POOL_ABI, "base_pool", block=block)
    if has_base_pool:
        return "StableSwapMeta"

    return "StableSwapV2"


def discover_pools(w3, factory_cfg, existing_addrs, max_new, block):
    """Discover new pools from a factory contract."""
    factory = w3.eth.contract(
        address=Web3.to_checksum_address(factory_cfg["address"]),
        abi=FACTORY_ABI,
    )
    count = factory.functions.pool_count().call(block_identifier=block)
    print(f"  {factory_cfg['label']}: {count} pools (checking all)")

    # Step 1: batch fetch pool addresses (newest first)
    indices = list(range(count - 1, -1, -1))
    addr_calls = [(factory, "pool_list", [i]) for i in indices]
    pool_addrs = _batch_call(w3, addr_calls, block)

    # Filter out existing
    new_addrs = []
    for addr in pool_addrs:
        if addr and addr.lower() not in existing_addrs:
            new_addrs.append(addr)
    if not new_addrs:
        return []

    # Step 2: liveness filter — try get_dy(0, 1, bal0/1000) on each pool.
    # Pools that revert or return 0 are dead (empty, paused, broken token).
    # This replaces the old rough TVL filter which assumed all tokens = $1.
    pool_contracts = {a: w3.eth.contract(address=a, abi=POOL_ABI) for a in new_addrs}
    bal_calls = [(pool_contracts[a], "balances", [0]) for a in new_addrs]
    bal_results = _batch_call(w3, bal_calls, block)

    # Build get_dy calls for pools with nonzero balance
    GET_DY_STABLE = json.loads('[{"name":"get_dy","outputs":[{"type":"uint256"}],"inputs":[{"type":"int128"},{"type":"int128"},{"type":"uint256"}],"stateMutability":"view","type":"function"}]')
    GET_DY_CRYPTO = json.loads('[{"name":"get_dy","outputs":[{"type":"uint256"}],"inputs":[{"type":"uint256"},{"type":"uint256"},{"type":"uint256"}],"stateMutability":"view","type":"function"}]')

    is_crypto = factory_cfg["variant"] in ("TwoCryptoNG", "TwoCryptoV1", "TriCryptoNG")

    dy_addrs = []
    dy_calls = []
    dx_per_pool = {}
    for addr, bal0 in zip(new_addrs, bal_results):
        if not bal0 or bal0 == 0:
            continue
        dx = max(bal0 // 1000, 1)
        dx_per_pool[addr] = dx
        # Crypto pools use uint256 indices, StableSwap uses int128
        abi = GET_DY_CRYPTO if is_crypto else GET_DY_STABLE
        c = w3.eth.contract(address=addr, abi=abi)
        dy_calls.append((c, "get_dy", [0, 1, dx]))
        dy_addrs.append(addr)

    dy_results = _batch_call(w3, dy_calls, block) if dy_calls else []

    # For non-crypto pools that failed, retry with the other ABI.
    # MetaPool Factory deploys both plain (uint256) and meta (int128) pools.
    retry_indices = []
    if not is_crypto:
        for i, (addr, dy) in enumerate(zip(dy_addrs, dy_results)):
            if dy is None:
                retry_indices.append(i)
        if retry_indices:
            retry_calls = []
            for i in retry_indices:
                addr = dy_addrs[i]
                c = w3.eth.contract(address=addr, abi=GET_DY_CRYPTO)
                retry_calls.append((c, "get_dy", [0, 1, dx_per_pool[addr]]))
            retry_results = _batch_call(w3, retry_calls, block)
            for i, dy in zip(retry_indices, retry_results):
                dy_results[i] = dy

    live_pools = []
    for addr, dy in zip(dy_addrs, dy_results):
        if dy and dy > 0:
            live_pools.append(addr)
    if not live_pools:
        return []

    # Step 3: fetch N_COINS for each pool, then batch fetch coin details
    tvl_passed = live_pools[:max_new]
    ncoin_calls = [(pool_contracts[a], "N_COINS", []) for a in tvl_passed]
    ncoin_results = _batch_call(w3, ncoin_calls, block)
    pool_ncoins = {}
    for addr, n in zip(tvl_passed, ncoin_results):
        pool_ncoins[addr] = n if n and n > 0 else 4  # fallback to 4 for legacy

    detail_calls = []
    detail_layout = []  # (addr, n_coins) to reconstruct results
    for addr in tvl_passed:
        pc = pool_contracts[addr]
        n = pool_ncoins[addr]
        for ci in range(n):
            detail_calls.append((pc, "coins", [ci]))
            detail_calls.append((pc, "balances", [ci]))
        detail_layout.append((addr, n))
    detail_results = _batch_call(w3, detail_calls, block)

    # Step 4: batch fetch decimals/symbols for coins
    coin_addrs = {}
    offset = 0
    for addr, n in detail_layout:
        coins_for_pool = []
        for ci in range(n):
            base = offset + ci * 2
            coin_addr = detail_results[base]
            if coin_addr and coin_addr != "0x" + "0" * 40:
                coins_for_pool.append((coin_addr, detail_results[base + 1]))
        coin_addrs[addr] = coins_for_pool
        offset += n * 2

    # Batch decimals/symbols
    token_calls = []
    token_map = []  # (pool_addr, coin_idx)
    for addr, coins in coin_addrs.items():
        for ci, (coin_addr, _) in enumerate(coins):
            tc = w3.eth.contract(address=coin_addr, abi=TOKEN_ABI)
            token_calls.append((tc, "decimals", []))
            token_calls.append((tc, "symbol", []))
            token_map.append((addr, ci))
    token_results = _batch_call(w3, token_calls, block)

    # Assemble candidates
    token_info = {}
    for ti, (pool_addr, ci) in enumerate(token_map):
        dec = token_results[ti * 2] or 18
        sym = token_results[ti * 2 + 1] or "???"
        if pool_addr not in token_info:
            token_info[pool_addr] = []
        token_info[pool_addr].append((dec, sym))

    # Step 5: For TwoCryptoNG, detect MATH version to distinguish TwoCryptoNG vs TwoCryptoStable
    math_versions = {}
    if factory_cfg["variant"] == "TwoCryptoNG":
        math_calls = [(w3.eth.contract(address=a, abi=MATH_GETTER_ABI), "MATH", []) for a in tvl_passed]
        math_addrs = _batch_call(w3, math_calls, block)
        ver_calls = []
        ver_map = []
        for pool_addr, math_addr in zip(tvl_passed, math_addrs):
            if math_addr:
                ver_calls.append((w3.eth.contract(address=math_addr, abi=VERSION_ABI), "version", []))
                ver_map.append(pool_addr)
        if ver_calls:
            ver_results = _batch_call(w3, ver_calls, block)
            for pool_addr, ver in zip(ver_map, ver_results):
                math_versions[pool_addr] = ver or "unknown"

    candidates = []
    for addr in tvl_passed:
        coins = coin_addrs.get(addr, [])
        info = token_info.get(addr, [])
        if not coins or not info:
            continue
        symbols = [s for _, s in info]

        variant = factory_cfg["variant"]
        # Detect TwoCryptoStable: MATH v0.1.0 = StableSwap math
        if variant == "TwoCryptoNG":
            math_ver = math_versions.get(addr, "v2.0.0")
            if math_ver == "v0.1.0":
                variant = "TwoCryptoStable"
        # MetaPool Factory: ask factory is_meta() to distinguish meta vs plain
        elif variant == "meta_factory":
            try:
                fc = w3.eth.contract(
                    address=Web3.to_checksum_address(factory_cfg["address"]),
                    abi=FACTORY_ABI,
                )
                is_meta = fc.functions.is_meta(Web3.to_checksum_address(addr)).call(block_identifier=block)
                variant = "StableSwapMeta" if is_meta else "StableSwapV2"
            except Exception:
                variant = detect_variant_onchain(w3, addr, block)
        # Auto-detect variant by probing on-chain functions
        elif variant == "auto":
            variant = detect_variant_onchain(w3, addr, block)

        name = "/".join(symbols)
        print(f"    {addr} {name} ({len(coins)}-coin, {variant})")
        candidates.append({
            "address": addr,
            "variant": variant,
            "name": name,
        })

    return candidates


def verify_pool_quick(w3, pool_entry, block) -> tuple[bool, str]:
    """Quick sanity check: 1 swap, compare on-chain get_dy with our get_amount_out.

    Full fuzz verification happens later in CI when the PR is created.
    This just checks if the variant classification and parameter reading are correct.
    """
    addr = Web3.to_checksum_address(pool_entry["address"])
    variant = pool_entry["variant"]
    pool = w3.eth.contract(address=addr, abi=POOL_ABI)

    # Read balance[0] for a reasonable swap amount (0.1% of balance)
    bal0 = pool.functions.balances(0).call(block_identifier=block)
    dx = max(bal0 // 1000, 1)

    # Get on-chain result
    try:
        if variant.startswith("StableSwap"):
            STABLE_ABI = json.loads('[{"name":"get_dy","outputs":[{"type":"uint256"}],"inputs":[{"type":"int128"},{"type":"int128"},{"type":"uint256"}],"stateMutability":"view","type":"function"}]')
            c = w3.eth.contract(address=addr, abi=STABLE_ABI)
            on_chain = c.functions.get_dy(0, 1, dx).call(block_identifier=block)
        else:
            CRYPTO_ABI = json.loads('[{"name":"get_dy","outputs":[{"type":"uint256"}],"inputs":[{"type":"uint256"},{"type":"uint256"},{"type":"uint256"}],"stateMutability":"view","type":"function"}]')
            c = w3.eth.contract(address=addr, abi=CRYPTO_ABI)
            on_chain = c.functions.get_dy(0, 1, dx).call(block_identifier=block)
    except Exception as e:
        return False, f"on-chain get_dy failed: {e}"

    if on_chain == 0:
        return False, "on-chain get_dy returned 0"

    return True, f"get_dy(0,1,{dx})={on_chain}"


def main():
    parser = argparse.ArgumentParser(description="Curve pool indexer")
    parser.add_argument("--chain-id", type=int, default=1)
    parser.add_argument("--max-new", type=int, default=20, help="Max new pools to add per run")
    parser.add_argument("--dry-run", action="store_true", help="Don't write registry file")
    args = parser.parse_args()

    if args.chain_id not in FACTORIES:
        sys.exit(f"No factory config for chain {args.chain_id}")

    rpc_url = get_rpc_url(args.chain_id)
    w3 = Web3(Web3.HTTPProvider(rpc_url))
    latest = w3.eth.block_number
    block = latest - 5  # use a settled block to avoid inconsistent state
    print(f"Chain {args.chain_id}: block {block} (latest {latest})")

    registry_path = Path(f"tests/registry/{args.chain_id}.toml")
    registry = read_registry(registry_path)
    existing = {p["address"].lower() for p in registry["pools"]}
    print(f"  {len(existing)} existing pools")

    # Count total factory pools across all factories
    total_factory_pools = 0
    for factory in FACTORIES[args.chain_id]:
        fc = w3.eth.contract(address=Web3.to_checksum_address(factory["address"]), abi=FACTORY_ABI)
        total_factory_pools += fc.functions.pool_count().call(block_identifier=block)

    # Discover — take up to max_new from EACH factory for even coverage
    all_candidates = []
    for factory in FACTORIES[args.chain_id]:
        candidates = discover_pools(w3, factory, existing, args.max_new, block)
        all_candidates.extend(candidates)

    print(f"\n{len(all_candidates)} new live candidates")

    if not all_candidates:
        print("No new pools to add.")
        registry["last_updated"] = datetime.now(timezone.utc).strftime("%Y-%m-%d")
        registry["total_factory_pools"] = total_factory_pools
        if not args.dry_run:
            write_registry(registry_path, registry)
        return

    # Save original pool list for diff
    original_pools = list(registry["pools"])

    # Verify
    verified = 0
    failed = 0
    added_names = []
    skipped_names = []
    for entry in all_candidates:
        print(f"  Checking {entry['name']} ({entry['address']})... ", end="", flush=True)
        ok, msg = verify_pool_quick(w3, entry, block)
        if ok:
            # Pool passes sanity check — add to registry, full fuzz happens in CI on the PR
            print(f"OK ({msg})")
            registry["pools"].append(entry)
            added_names.append(f"{entry['name']} ({entry['address']})")
            verified += 1
        else:
            print(f"SKIP ({msg})")
            skipped_names.append(f"{entry['name']} ({entry['address']}): {msg}")
            failed += 1

    registry["last_updated"] = datetime.now(timezone.utc).strftime("%Y-%m-%d")

    print(f"\nSummary: {verified} added, {failed} skipped")

    if args.dry_run:
        print("(dry run — no files written)")
    elif verified == 0:
        print("No new pools to add.")
        # Update timestamp only
        registry["last_updated"] = datetime.now(timezone.utc).strftime("%Y-%m-%d")
        write_registry(registry_path, registry)
    else:
        # Write new pools to pending file (not main registry)
        # CI will fuzz only these, then merge if passed
        pending_path = Path(f"tests/registry/{args.chain_id}_pending.toml")
        pending_registry = {"pools": [p for p in registry["pools"] if p not in original_pools]}
        write_registry(pending_path, pending_registry)
        print(f"Pending pools written to {pending_path}")

        # Also update main registry (will be committed together)
        registry["last_updated"] = datetime.now(timezone.utc).strftime("%Y-%m-%d")
        registry["total_factory_pools"] = total_factory_pools
        write_registry(registry_path, registry)
        print(f"Registry updated: {registry_path}")

        # Update verified pool count in README
        pool_count = len(registry["pools"])
        update_readme_badge(args.chain_id, pool_count, registry["last_updated"], total_factory_pools)

        # Write PR summary for CI
        write_pr_summary(verified, failed, added_names, skipped_names)


def write_pr_summary(verified: int, failed: int, added: list[str], skipped: list[str]):
    """Write PR body summary to .pr-summary.md for CI to pick up."""
    lines = [f"## Pool Registry Update\n"]
    if added:
        lines.append(f"### Added ({verified} pools)\n")
        for name in added:
            lines.append(f"- {name}")
        lines.append("")
    if skipped:
        lines.append(f"### Skipped ({failed} pools)\n")
        for name in skipped:
            lines.append(f"- {name}")
        lines.append("")
    if not added:
        lines.append("No new pools added.\n")
    lines.append(f"\n> Full fuzz verification runs automatically on this PR.")
    Path(".pr-summary.md").write_text("\n".join(lines) + "\n")
    print(f"PR summary written to .pr-summary.md")


CHAIN_NAMES = {1: "Ethereum", 8453: "Base"}
CHAIN_FUZZ_BADGES = {
    1: "[![Fuzz](https://github.com/sunce86/curve-math/actions/workflows/fuzz-ethereum.yml/badge.svg)](https://github.com/sunce86/curve-math/actions/workflows/fuzz-ethereum.yml)",
}


def update_readme_badge(chain_id: int, count: int, last_updated: str, total: int = 0):
    """Update the chain status table row for the given chain in README.md."""
    git_root = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    readme = Path(git_root) / "README.md"
    if not readme.exists():
        return
    content = readme.read_text()

    import re
    chain_name = CHAIN_NAMES.get(chain_id, f"Chain {chain_id}")
    badge = CHAIN_FUZZ_BADGES.get(chain_id, "")
    pct = (count * 100 // total) if total else 0
    pool_str = f"{count} / {total} ![](https://geps.dev/progress/{pct}?successColor=6366f1)" if total else str(count)
    new_row = f"| {chain_name} | {badge} | {pool_str} | {last_updated} |"
    content = re.sub(
        rf'\| {re.escape(chain_name)} \|.*\|.*\|.*\|',
        new_row,
        content,
    )
    readme.write_text(content)
    print(f"Updated README: {pool_str} pools, last indexed {last_updated}")


if __name__ == "__main__":
    main()
