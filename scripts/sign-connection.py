#!/usr/bin/env python3
"""Issue a GenroApp connection with an Ed25519 PEM signing key (OpenSSL 3)."""
import argparse
import base64
import json
import subprocess
import tempfile
from pathlib import Path
from urllib.parse import urlsplit

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for arg in ("key", "name", "url", "output"):
        parser.add_argument("--" + arg, required=True)
    parser.add_argument("--id", help="Stable application identifier within this origin")
    parser.add_argument("--icon", type=Path, help="32×32 PNG menu icon")
    args = parser.parse_args()
    url = urlsplit(args.url)
    if url.scheme != "https" or not url.hostname or url.username is not None or url.password is not None:
        parser.error("Expected an HTTPS URL without credentials")
    document = {"format": "it.softwell.runner.link/v1", "name": args.name, "url": args.url}
    if args.id:
        document["id"] = args.id
    if args.icon:
        icon = args.icon.read_bytes()
        if not icon.startswith(b"\x89PNG\r\n\x1a\n") or icon[16:24] != bytes.fromhex("0000002000000020"):
            parser.error("Expected a 32×32 PNG icon")
        document["icon"] = base64.b64encode(icon).decode("ascii")
    payload = json.dumps(document, ensure_ascii=False).encode()
    with tempfile.NamedTemporaryFile() as message:
        message.write(payload)
        message.flush()
        signature = subprocess.check_output(["openssl", "pkeyutl", "-sign", "-rawin",
                                             "-inkey", args.key, "-in", message.name])
    encode = lambda value: base64.b64encode(value).decode("ascii")
    Path(args.output).write_text(json.dumps({"payload": encode(payload), "signature": encode(signature)}))

if __name__ == "__main__":
    main()
