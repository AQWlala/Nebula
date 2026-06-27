#!/usr/bin/env python3
# generate-updater-key.py — produce a Tauri-compatible Ed25519
# keypair for the auto-updater.
#
# P0#11 fix: `tauri.conf.json::plugins.updater.pubkey` was set to
# the literal placeholder string "REPLACE_WITH_RELEASE_SIGNING_PUBLIC_KEY",
# which makes the in-app update check always fail signature
# verification. This script generates a real Ed25519 keypair in the
# same byte layout that `ed25519-dalek` (the crate the Tauri updater
# is built on) uses, and writes:
#
#   * `keys/updater_public.b64`     — public key, raw 32 bytes,
#                                     base64 (Standard alphabet).
#                                     This is the value that must
#                                     replace the placeholder in
#                                     tauri.conf.json.
#   * `keys/updater_private.b64`    — private key, raw 32 bytes
#                                     (the *seed* — ed25519-dalek
#                                     expects the seed, not the
#                                     expanded form), base64.
#                                     This is the value that must
#                                     be stored in the
#                                     `TAURI_SIGNING_PRIVATE_KEY`
#                                     GitHub Actions secret.
#   * `keys/updater_private_password.b64` — a random 32-byte
#                                     password, base64. Store in
#                                     `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
#                                     (the official
#                                     `tauri signer generate` flow
#                                     uses one too).
#
# The output files are **not** committed to git (see `.gitignore`).
# Re-running the script overwrites them.
#
# Usage:
#   python scripts/generate-updater-key.py
#   python scripts/generate-updater-key.py --out keys
#
# NOTE: This script uses the Python `cryptography` library to drive
# the same Ed25519 primitive as Rust's `ed25519-dalek`. The two
# libraries are wire-compatible at the raw-byte level.

from __future__ import annotations

import argparse
import base64
import os
import secrets
from pathlib import Path

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization


def b64(data: bytes) -> str:
    """Standard base64 with newlines stripped (matches Tauri CLI)."""
    return base64.standard_b64encode(data).decode("ascii").replace("\n", "")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", default="keys", help="output directory (default: keys)")
    args = parser.parse_args()

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    sk = Ed25519PrivateKey.generate()
    pk = sk.public_key()

    # 32-byte Ed25519 seed (what ed25519-dalek expects in its
    # `SigningKey::from_bytes` constructor).
    seed_bytes = sk.private_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PrivateFormat.Raw,
        encryption_algorithm=serialization.NoEncryption(),
    )
    # 32-byte Ed25519 public key.
    pub_bytes = pk.public_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PublicFormat.Raw,
    )

    assert len(seed_bytes) == 32
    assert len(pub_bytes) == 32

    pub_b64 = b64(pub_bytes)
    priv_b64 = b64(seed_bytes)
    password_b64 = b64(secrets.token_bytes(32))

    (out / "updater_public.b64").write_text(pub_b64 + "\n", encoding="utf-8")
    (out / "updater_private.b64").write_text(priv_b64 + "\n", encoding="utf-8")
    (out / "updater_private_password.b64").write_text(password_b64 + "\n", encoding="utf-8")

    print("Tauri updater Ed25519 keypair generated.")
    print()
    print("Public key (paste into tauri.conf.json::plugins.updater.pubkey):")
    print(f"  {pub_b64}")
    print()
    print(f"Private key      -> {out / 'updater_private.b64'} (32 bytes, base64)")
    print(f"Private key pass -> {out / 'updater_private_password.b64'} (32 bytes, base64)")
    print(f"Public  key      -> {out / 'updater_public.b64'} (32 bytes, base64)")
    print()
    print("Next steps:")
    print("  1. Copy the public key into tauri.conf.json.")
    print("  2. Store the private key + password as GitHub Actions secrets")
    print("     (TAURI_SIGNING_PRIVATE_KEY, TAURI_SIGNING_PRIVATE_KEY_PASSWORD).")
    print("  3. Add 'keys/' to .gitignore if it isn't already.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
