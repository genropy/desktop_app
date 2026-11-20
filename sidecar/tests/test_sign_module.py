"""scripts/sign-module.py builds the package the shell verifies."""
import base64
import hashlib
import json
import subprocess
import sys
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[2] / 'scripts' / 'sign-module.py'


def openssl(*args):
    return subprocess.run(['openssl', *args], check=True, capture_output=True)


def test_the_package_carries_a_signed_manifest_of_every_file(tmp_path):
    key, public = tmp_path / 'key.pem', tmp_path / 'public.pem'
    openssl('genpkey', '-algorithm', 'ed25519', '-out', str(key))
    openssl('pkey', '-in', str(key), '-pubout', '-out', str(public))
    source = tmp_path / 'source'
    (source / 'scale' / '__pycache__').mkdir(parents=True)
    (source / 'scale' / '__init__.py').write_text('X = 1\n')
    (source / 'scale' / '__pycache__' / 'x.cpython-314.pyc').write_bytes(b'cache')
    (source / 'serial').mkdir()
    (source / 'serial' / '__init__.py').write_text('Y = 2\n')
    subprocess.run([sys.executable, str(SCRIPT), '--key', str(key), '--name', 'scale', '--version', '1.0.0',
                    '--origin', 'https://App.Acme.com', '--source', str(source), '--output', str(tmp_path / 'out')],
                   check=True, capture_output=True)
    target = tmp_path / 'out' / 'scale' / '1.0.0'
    envelope = json.loads((target / 'manifest.json').read_text())
    payload = base64.b64decode(envelope['payload'])
    (tmp_path / 'payload').write_bytes(payload)
    (tmp_path / 'signature').write_bytes(base64.b64decode(envelope['signature']))
    openssl('pkeyutl', '-verify', '-rawin', '-pubin', '-inkey', str(public),
            '-in', str(tmp_path / 'payload'), '-sigfile', str(tmp_path / 'signature'))
    manifest = json.loads(payload)
    assert manifest['origins'] == ['https://app.acme.com']
    assert sorted(manifest['files']) == ['scale/__init__.py', 'serial/__init__.py']
    for path, digest in manifest['files'].items():
        assert hashlib.sha256((target / path).read_bytes()).hexdigest() == digest
    assert not (target / 'scale' / '__pycache__').exists()
