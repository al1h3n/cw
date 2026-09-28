"""Run on the publisher's build machine, never on the update mirror.

Requires: python -m pip install cryptography
"""

import argparse
import hashlib
import json
from pathlib import Path
import secrets
import subprocess

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    keygen = sub.add_parser("keygen")
    keygen.add_argument("private_key", type=Path)
    sign = sub.add_parser("sign")
    sign.add_argument("private_key", type=Path)
    sign.add_argument("version")
    sign.add_argument("exe", type=Path)
    sign.add_argument("output_dir", type=Path)
    args = parser.parse_args()

    if args.command == "keygen":
        secret = secrets.token_bytes(32)
        with args.private_key.open("x", encoding="ascii") as file:
            file.write(secret.hex() + "\n")
        key = Ed25519PrivateKey.from_private_bytes(secret)
        public = key.public_key().public_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PublicFormat.Raw,
        )
        print("PUBLIC KEY:", public.hex())
        return

    secret = bytes.fromhex(args.private_key.read_text(encoding="ascii").strip())
    if len(secret) != 32:
        raise ValueError("private key must be 32 bytes")
    key = Ed25519PrivateKey.from_private_bytes(secret)
    reported = subprocess.check_output([str(args.exe.resolve()), "version"], text=True).strip()
    if not reported.endswith("agent " + args.version):
        raise ValueError(f"executable reports {reported!r}, expected agent {args.version}")
    binary = args.exe.read_bytes()
    asset = "cowatcher-agent.exe"
    digest = hashlib.sha256(binary).hexdigest()
    signed = f"cowatcher-agent-update-v1\n{args.version}\n{asset}\n{digest}\n".encode()
    manifest = {
        "version": args.version,
        "asset": asset,
        "sha256": digest,
        "signature": key.sign(signed).hex(),
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    (args.output_dir / asset).write_bytes(binary)
    temp = args.output_dir / "manifest.json.tmp"
    temp.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    temp.replace(args.output_dir / "manifest.json")
    print("Published", args.version, "to", args.output_dir)


if __name__ == "__main__":
    main()
