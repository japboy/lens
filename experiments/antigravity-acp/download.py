#!/usr/bin/env python3
"""Fetch the version-pinned official macOS arm64 archive into a new temp directory."""
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import tempfile
import urllib.request
import zipfile

URL = 'https://dl.google.com/agy-extensions/releases/macos/agy-acp-server-1.2.1-darwin-arm64.zip'
EXPECTED_SHA256 = '0fab9938812e6b32b3b543e65e4f3a0025ceef755413db13542d9a9b81ea803c'
if platform.system() != 'Darwin' or platform.machine() != 'arm64':
    raise SystemExit('This experiment is pinned to macOS arm64.')
root = Path(tempfile.mkdtemp(prefix='lens-antigravity-131-'))
archive = root / 'adapter.zip'
with urllib.request.urlopen(URL, timeout=60) as response:
    archive.write_bytes(response.read())
if hashlib.sha256(archive.read_bytes()).hexdigest() != EXPECTED_SHA256:
    raise SystemExit('Archive differs from the observed pinned artifact; review before execution.')
runtime = root / 'runtime'
runtime.mkdir()
with zipfile.ZipFile(archive) as source:
    for name in ['agy_acp_server.par', 'localharness_external']:
        entries = [entry for entry in source.infolist() if entry.filename == name]
        if len(entries) != 1:
            raise SystemExit('Unexpected archive layout: ' + name)
        target = runtime / name
        target.write_bytes(source.read(entries[0]))
        target.chmod(0o700)
manifest = {'url': URL, 'archive_sha256': EXPECTED_SHA256, 'binaries': {name: hashlib.sha256((runtime / name).read_bytes()).hexdigest() for name in ['agy_acp_server.par', 'localharness_external']}, 'os': subprocess.check_output(['sw_vers']).decode(), 'architecture': platform.machine()}
(root / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
print(str(root))
