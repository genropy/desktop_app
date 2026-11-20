#!/usr/bin/env python3
"""Build a GenroApp module package signed with an Ed25519 PEM signing key (OpenSSL 3).

`--source` holds what goes to the computer: the package `<name>/` and the pure Python
libraries it needs, side by side. The output is `<output>/<name>/<version>/` with the
files and `manifest.json`, the signed manifest. A site serves that tree under
`/_tools/modules/` and lists the `manifest.json` of its modules, as a JSON array, at
`/_tools/modules`.
"""
import argparse
import base64
import hashlib
import json
import re
import shutil
import subprocess
import tempfile
from pathlib import Path
from urllib.parse import urlsplit

FORMAT = "it.softwell.genroapp.module/v1"
BUILTIN = {"system", "inbox"}
SKIPPED = {"__pycache__", ".DS_Store"}


def source_files(source):
    "Relative paths with `/` of the files under `source`, without bytecode caches."
    for path in sorted(source.rglob("*")):
        relative = path.relative_to(source)
        if path.is_file() and not SKIPPED.intersection(relative.parts) and path.suffix != ".pyc":
            yield relative.as_posix()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    for arg in ("key", "name", "version", "source", "output"):
        parser.add_argument("--" + arg, required=True)
    parser.add_argument("--origin", action="append", required=True, help="HTTPS origin allowed to call the module")
    args = parser.parse_args()
    if not re.fullmatch(r"[a-z][a-z0-9_]{0,63}", args.name) or args.name in BUILTIN:
        parser.error("The name must be a lowercase Python package name, not system or inbox")
    if not re.fullmatch(r"[0-9A-Za-z][0-9A-Za-z.+-]{0,63}", args.version):
        parser.error("Invalid version")
    origins = []
    for origin in args.origin:
        url = urlsplit(origin)
        if url.scheme != "https" or not url.hostname or url.path not in ("", "/") or url.query or url.username:
            parser.error("Expected an HTTPS origin: %s" % origin)
        origins.append("https://" + url.netloc.lower())
    source = Path(args.source)
    if not (source / args.name / "__init__.py").is_file() and not (source / (args.name + ".py")).is_file():
        parser.error("The source has no package %s" % args.name)
    files = {path: hashlib.sha256((source / path).read_bytes()).hexdigest() for path in source_files(source)}
    if "manifest.json" in files:
        parser.error("The source must not contain manifest.json at its top")
    manifest = {"format": FORMAT, "name": args.name, "version": args.version, "origins": origins, "files": files}
    payload = json.dumps(manifest, ensure_ascii=False).encode()
    with tempfile.NamedTemporaryFile() as message:
        message.write(payload)
        message.flush()
        signature = subprocess.check_output(["openssl", "pkeyutl", "-sign", "-rawin",
                                             "-inkey", args.key, "-in", message.name])
    target = Path(args.output) / args.name / args.version
    if target.exists():
        parser.error("%s exists already" % target)
    for path in files:
        (target / path).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source / path, target / path)
    envelope = {"payload": base64.b64encode(payload).decode("ascii"),
                "signature": base64.b64encode(signature).decode("ascii")}
    (target / "manifest.json").write_text(json.dumps(envelope))
    print(target)


if __name__ == "__main__":
    main()
