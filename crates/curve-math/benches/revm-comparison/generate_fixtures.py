#!/usr/bin/env python3
"""
Generate benchmark fixtures by pre-fetching pool bytecodes and storage slots.

Uses debug_traceCall to discover which storage slots get_dy reads,
then saves everything needed to reproduce the call in revm without RPC.

Usage:
    pip install web3
    RPC_URL_1=<url> python generate_fixtures.py
"""

import json
import os
import sys
from pathlib import Path

try:
    from web3 import Web3
except ImportError:
    sys.exit("pip install web3")


def get_rpc():
    url = os.environ.get("RPC_URL_1") or os.environ.get("RPC_URL")
    if not url:
        sys.exit("RPC_URL_1 or RPC_URL required")
    return url


# Benchmark pools: one per solver class
POOLS = [
    {
        "name": "3pool",
        "label": "StableSwap (2-coin Newton)",
        "address": "0xbEbc44782C7dB0a1A60Cb6fe97d0b483032FF1C7",
        "variant": "StableSwapV1",
        "i": 0, "j": 1,
        "dx": 1000 * 10**18,  # 1000 DAI
        "get_dy_sig": "get_dy(int128,int128,uint256)",
    },
    {
        "name": "sUSDS_USDT",
        "label": "StableSwapNG (oracle + dynamic fee)",
        "address": "0x00836Fe54625BE242BcFA286207795405ca4fD10",
        "variant": "StableSwapNG",
        "i": 0, "j": 1,
        "dx": 1000 * 10**18,  # 1000 sUSDS
        "get_dy_sig": "get_dy(int128,int128,uint256)",
    },
    {
        "name": "crvUSD_FXN",
        "label": "TwoCryptoNG (Cardano cubic solver)",
        "address": "0xfb8b95Fb2296a0Ad4b6b1419fdAA5AA5F13e4009",
        "variant": "TwoCryptoNG",
        "i": 0, "j": 1,
        "dx": 100 * 10**18,  # 100 crvUSD
        "get_dy_sig": "get_dy(uint256,uint256,uint256)",
    },
    {
        "name": "crvUSD_WETH_CRV",
        "label": "TriCryptoNG (3-coin hybrid solver)",
        "address": "0x4eBdF703948ddCEA3B11f675B4D1Fba9d2414A14",
        "variant": "TriCryptoNG",
        "i": 0, "j": 1,
        "dx": 1000 * 10**18,  # 1000 crvUSD
        "get_dy_sig": "get_dy(uint256,uint256,uint256)",
    },
]


POOL_ABI = json.loads('[{"name":"coins","outputs":[{"type":"address"}],"inputs":[{"type":"uint256"}],"stateMutability":"view","type":"function"},{"name":"balances","outputs":[{"type":"uint256"}],"inputs":[{"type":"uint256"}],"stateMutability":"view","type":"function"},{"name":"A","outputs":[{"type":"uint256"}],"inputs":[],"stateMutability":"view","type":"function"},{"name":"fee","outputs":[{"type":"uint256"}],"inputs":[],"stateMutability":"view","type":"function"}]')
CRYPTO_ABI = json.loads('[{"name":"gamma","outputs":[{"type":"uint256"}],"inputs":[],"stateMutability":"view","type":"function"},{"name":"D","outputs":[{"type":"uint256"}],"inputs":[],"stateMutability":"view","type":"function"},{"name":"price_scale","outputs":[{"type":"uint256"}],"inputs":[],"stateMutability":"view","type":"function"},{"name":"mid_fee","outputs":[{"type":"uint256"}],"inputs":[],"stateMutability":"view","type":"function"},{"name":"out_fee","outputs":[{"type":"uint256"}],"inputs":[],"stateMutability":"view","type":"function"},{"name":"fee_gamma","outputs":[{"type":"uint256"}],"inputs":[],"stateMutability":"view","type":"function"}]')
CRYPTO3_ABI = json.loads('[{"name":"price_scale","outputs":[{"type":"uint256"}],"inputs":[{"type":"uint256"}],"stateMutability":"view","type":"function"}]')
OFFPEG_ABI = json.loads('[{"name":"offpeg_fee_multiplier","outputs":[{"type":"uint256"}],"inputs":[],"stateMutability":"view","type":"function"},{"name":"stored_rates","outputs":[{"type":"uint256[]"}],"inputs":[],"stateMutability":"view","type":"function"}]')
TOKEN_ABI = json.loads('[{"name":"decimals","outputs":[{"type":"uint8"}],"inputs":[],"stateMutability":"view","type":"function"}]')


def read_pool_params(w3, addr, variant, block):
    """Read pool parameters needed for curve-math Pool construction."""
    c = w3.eth.contract(address=addr, abi=POOL_ABI)
    A = c.functions.A().call(block_identifier=block)

    # Read coins and decimals
    coins = []
    decimals = []
    for i in range(4):
        try:
            coin = c.functions.coins(i).call(block_identifier=block)
            if coin == "0x" + "0" * 40:
                break
            coins.append(coin)
            tc = w3.eth.contract(address=coin, abi=TOKEN_ABI)
            decimals.append(tc.functions.decimals().call())
        except:
            break
    n_coins = len(coins)

    balances = []
    for i in range(n_coins):
        balances.append(str(c.functions.balances(i).call(block_identifier=block)))

    params = {
        "A": str(A),
        "n_coins": n_coins,
        "decimals": decimals,
        "balances": balances,
    }

    if variant in ("StableSwapV0", "StableSwapV1"):
        params["fee"] = str(c.functions.fee().call(block_identifier=block))
        params["rates"] = [str(10 ** (36 - d)) for d in decimals]
    elif variant == "StableSwapV2":
        params["fee"] = str(c.functions.fee().call(block_identifier=block))
        params["rates"] = [str(10 ** (36 - d)) for d in decimals]
        params["A"] = str(A * 100)  # A_PRECISION
    elif variant == "StableSwapNG":
        params["fee"] = str(c.functions.fee().call(block_identifier=block))
        params["A"] = str(A * 100)  # A_PRECISION
        # offpeg_fee_multiplier: v5+ crvUSD factory pools lack this
        co = w3.eth.contract(address=addr, abi=OFFPEG_ABI)
        try:
            params["offpeg_fee_multiplier"] = str(co.functions.offpeg_fee_multiplier().call(block_identifier=block))
        except Exception:
            params["offpeg_fee_multiplier"] = str(10_000_000_000)  # FEE_DENOMINATOR default
        # stored_rates: v6+ crvUSD factory pools lack this, use decimal-based rates
        try:
            rates = co.functions.stored_rates().call(block_identifier=block)
            params["rates"] = [str(r) for r in rates]
        except Exception:
            params["rates"] = [str(10 ** (36 - d)) for d in decimals]
    elif variant in ("TwoCryptoNG", "TwoCryptoV1"):
        cc = w3.eth.contract(address=addr, abi=CRYPTO_ABI)
        params["gamma"] = str(cc.functions.gamma().call(block_identifier=block))
        params["D"] = str(cc.functions.D().call(block_identifier=block))
        params["price_scale"] = str(cc.functions.price_scale().call(block_identifier=block))
        params["mid_fee"] = str(cc.functions.mid_fee().call(block_identifier=block))
        params["out_fee"] = str(cc.functions.out_fee().call(block_identifier=block))
        params["fee_gamma"] = str(cc.functions.fee_gamma().call(block_identifier=block))
        params["precisions"] = [str(10 ** (18 - d)) for d in decimals]
    elif variant == "TriCryptoNG":
        cc = w3.eth.contract(address=addr, abi=CRYPTO_ABI)
        c3 = w3.eth.contract(address=addr, abi=CRYPTO3_ABI)
        params["gamma"] = str(cc.functions.gamma().call(block_identifier=block))
        params["D"] = str(cc.functions.D().call(block_identifier=block))
        ps0 = c3.functions.price_scale(0).call(block_identifier=block)
        ps1 = c3.functions.price_scale(1).call(block_identifier=block)
        params["price_scale"] = [str(ps0), str(ps1)]
        params["mid_fee"] = str(cc.functions.mid_fee().call(block_identifier=block))
        params["out_fee"] = str(cc.functions.out_fee().call(block_identifier=block))
        params["fee_gamma"] = str(cc.functions.fee_gamma().call(block_identifier=block))
        params["precisions"] = [str(10 ** (18 - d)) for d in decimals]

    return params


def encode_get_dy(sig, i, j, dx):
    """Encode get_dy calldata."""
    selector = Web3.keccak(text=sig)[:4]
    if "int128" in sig:
        import eth_abi
        return selector + eth_abi.encode(["int128", "int128", "uint256"], [i, j, dx])
    else:
        import eth_abi
        return selector + eth_abi.encode(["uint256", "uint256", "uint256"], [i, j, dx])


def trace_call(w3, to, data, block):
    """Use debug_traceCall to get all accessed storage slots."""
    result = w3.provider.make_request("debug_traceCall", [
        {"to": to, "data": "0x" + data.hex()},
        hex(block),
        {"tracer": "prestateTracer", "tracerConfig": {"diffMode": False}}
    ])
    if "error" in result:
        # Fallback: try without tracer (some RPCs don't support debug_traceCall)
        return None
    return result.get("result", {})


def get_storage_via_access_list(w3, to, data, block):
    """Use eth_createAccessList to discover storage slots."""
    result = w3.provider.make_request("eth_createAccessList", [
        {"to": to, "data": "0x" + data.hex()},
        hex(block),
    ])
    if "error" in result:
        return None
    access_list = result.get("result", {}).get("accessList", [])
    return access_list


def fetch_fixture(w3, pool_cfg, block):
    """Fetch all data needed for a benchmark fixture."""
    addr = Web3.to_checksum_address(pool_cfg["address"])
    calldata = encode_get_dy(pool_cfg["get_dy_sig"], pool_cfg["i"], pool_cfg["j"], pool_cfg["dx"])

    # Get expected result
    dy_abi = json.loads(f'[{{"name":"get_dy","outputs":[{{"type":"uint256"}}],"inputs":[{{"type":"{"int128" if "int128" in pool_cfg["get_dy_sig"] else "uint256"}"}},{{"type":"{"int128" if "int128" in pool_cfg["get_dy_sig"] else "uint256"}"}},{{"type":"uint256"}}],"stateMutability":"view","type":"function"}}]')
    c = w3.eth.contract(address=addr, abi=dy_abi)
    if "int128" in pool_cfg["get_dy_sig"]:
        expected_dy = c.functions.get_dy(pool_cfg["i"], pool_cfg["j"], pool_cfg["dx"]).call(block_identifier=block)
    else:
        expected_dy = c.functions.get_dy(pool_cfg["i"], pool_cfg["j"], pool_cfg["dx"]).call(block_identifier=block)

    print(f"  Expected dy: {expected_dy}")

    # Get access list (which contracts + storage slots are touched)
    access_list = get_storage_via_access_list(w3, addr, calldata, block)
    if access_list is None:
        print("  WARNING: eth_createAccessList not supported, using fallback")
        access_list = [{"address": addr.lower(), "storageKeys": []}]

    # Fetch bytecode and storage for each accessed contract
    accounts = {}
    for entry in access_list:
        contract_addr = Web3.to_checksum_address(entry["address"])
        code = w3.eth.get_code(contract_addr, block_identifier=block)
        storage = {}
        slots = entry.get("storageKeys") or []
        for slot in slots:
            val = w3.eth.get_storage_at(contract_addr, slot, block_identifier=block)
            storage[slot] = "0x" + val.hex()

        accounts[contract_addr.lower()] = {
            "code": "0x" + code.hex(),
            "storage": storage,
        }
        print(f"  Contract {contract_addr}: {len(code)} bytes code, {len(storage)} storage slots")

    # Read pool params for curve-math Pool construction
    pool_params = read_pool_params(w3, addr, pool_cfg["variant"], block)

    return {
        "name": pool_cfg["name"],
        "label": pool_cfg["label"],
        "variant": pool_cfg["variant"],
        "pool_address": addr.lower(),
        "calldata": "0x" + calldata.hex(),
        "i": pool_cfg["i"],
        "j": pool_cfg["j"],
        "dx": str(pool_cfg["dx"]),
        "expected_dy": str(expected_dy),
        "block": block,
        "pool_params": pool_params,
        "accounts": accounts,
    }


def main():
    w3 = Web3(Web3.HTTPProvider(get_rpc()))
    block = w3.eth.block_number - 5
    print(f"Block: {block}\n")

    fixtures_dir = Path(__file__).parent / "fixtures"
    fixtures_dir.mkdir(exist_ok=True)

    for pool_cfg in POOLS:
        print(f"Generating fixture for {pool_cfg['name']} ({pool_cfg['label']})...")
        try:
            fixture = fetch_fixture(w3, pool_cfg, block)
            out_path = fixtures_dir / f"{pool_cfg['name']}.json"
            with open(out_path, "w") as f:
                json.dump(fixture, f, indent=2)
            print(f"  → {out_path}\n")
        except Exception as e:
            print(f"  ERROR: {e}\n")


if __name__ == "__main__":
    main()
