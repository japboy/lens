#!/usr/bin/env python3
"""Explicitly authorize the official adapter's personal Google OAuth flow."""
import argparse
from pathlib import Path
import tempfile
from probe import Rpc

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, required=True)
args = parser.parse_args()
rpc = Rpc(args.binary, tempfile.mkdtemp(prefix='lens-antigravity-auth-'))
try:
    initialized = rpc.call('initialize', {'protocolVersion': 1, 'clientInfo': {'name': 'lens-antigravity-feasibility', 'version': '0.1.0'}, 'clientCapabilities': {}}, 30)
    if 'result' not in initialized:
        raise SystemExit('Initialization failed; no authentication started.')
    print('The official adapter will reuse existing auth or open Google sign-in. Complete sign-in in your browser. This saves provider credentials/settings through the adapter.', flush=True)
    result = rpc.call('authenticate', {'methodId': 'oauth-personal'}, 300)
    if 'result' not in result:
        raise SystemExit('Authentication did not complete; no credential data is printed.')
    print('Authentication completed.')
finally:
    rpc.close()
