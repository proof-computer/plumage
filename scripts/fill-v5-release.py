#!/usr/bin/env python3
"""Bind the committed V5 policy draft to one built release.

Writes `release.artifact.digest` and the five literal payload variables
`plumage launch` reads. Every value comes from the payload workflow's
`payload-identity.json` or from the three single-part arguments. Nothing
is fetched, guessed or published. The script refuses to write a file that
still has the all-zero digest or an empty payload variable.
"""
import argparse
import json
import re
import sys
from pathlib import Path
from urllib.parse import urlsplit

VARIABLES = (
    'LISKOV_PLUMAGE_PAYLOAD_URL',
    'LISKOV_PLUMAGE_PAYLOAD_SHA256',
    'LISKOV_PLUMAGE_PAYLOAD_BYTES',
    'LISKOV_PLUMAGE_PAYLOAD_PART_SHA256',
    'LISKOV_PLUMAGE_PAYLOAD_PART_BYTES',
)
CONNECTOR_TOKEN_ENV = 'LISKOV_PLUMAGE_CLOUDFLARED_TOKEN'
ZERO_DIGEST = 'sha256:' + ('0' * 64)
MAX_PAYLOAD_BYTES = 1024 * 1024 * 1024
MAX_PARTS = 64


def digest(value):
    if not re.fullmatch(r'sha256:[0-9a-f]{64}', value):
        raise argparse.ArgumentTypeError('artifact digest must be sha256: plus 64 lowercase hex digits')
    return value


def hex64(value):
    if not isinstance(value, str) or not re.fullmatch(r'[0-9a-f]{64}', value):
        raise ValueError('payload sha256 must be 64 lowercase hex digits')
    return value


def size(value):
    number = int(value)
    if not 1 <= number <= MAX_PAYLOAD_BYTES:
        raise ValueError('payload bytes must be 1..1 GiB')
    return number


def https_url(value):
    parts = urlsplit(value)
    if parts.scheme != 'https' or not parts.hostname or parts.username or parts.password or parts.query or parts.fragment or any(c.isspace() for c in value):
        raise ValueError('payload URL must be HTTPS without credentials, query, fragment or whitespace')
    return value


def identity_from_file(path):
    document = json.loads(path.read_text())
    if document.get('format') != 'plumage.payload-identity.v1':
        raise ValueError('payload identity must be plumage.payload-identity.v1')
    parts = document.get('parts')
    if not isinstance(parts, list) or not 1 <= len(parts) <= MAX_PARTS:
        raise ValueError('payload identity needs 1..64 parts')
    connector = document.get('cloudflared')
    if connector is not None and (not isinstance(connector, dict) or connector.get('token_env') != CONNECTOR_TOKEN_ENV):
        raise ValueError('payload identity names an unknown connector token variable')
    identity = {
        'connector': connector is not None,
        'sha256': hex64(document.get('sha256')),
        'bytes': size(document.get('bytes')),
        'parts': [{'url': https_url(part['url']), 'sha256': hex64(part['sha256']), 'bytes': size(part['bytes'])} for part in parts],
    }
    if sum(part['bytes'] for part in identity['parts']) != identity['bytes']:
        raise ValueError('payload part sizes do not sum to the archive size')
    return identity


def assert_filled(document):
    artifact = document.get('release', {}).get('artifact', {}).get('digest', '')
    if artifact == ZERO_DIGEST:
        raise ValueError('refusing to write a policy that still has the all-zero artifact digest')
    values = {item.get('name'): item.get('value') for item in document.get('configuration', {}).get('variables', [])}
    for name in VARIABLES:
        value = values.get(name)
        if not isinstance(value, str) or not value.strip():
            raise ValueError(f'refusing to write a policy that still has an empty payload variable ({name})')


def fill(document, artifact_digest, identity):
    if document.get('schema') != 'proof.liskov.application-manifest' or document.get('schemaVersion') != 5:
        raise ValueError('only the retained V5 manifest pair is filled')
    release = document.setdefault('release', {})
    if release.get('mode') != 'pinned':
        raise ValueError('the draft must already be a pinned release')
    release['artifact'] = {'digest': artifact_digest}
    # A payload that starts the connector refuses to launch without its token,
    # so the policy must already deliver it. The secret id is the owner's to
    # choose; this script never writes one.
    if identity.get('connector'):
        secrets = document.get('configuration', {}).get('secrets') or []
        delivered = any(
            isinstance(item, dict) and isinstance(item.get('destination'), dict)
            and item['destination'].get('kind') == 'environment' and item['destination'].get('name') == CONNECTOR_TOKEN_ENV
            for item in secrets)
        if not delivered:
            raise ValueError(f'this payload starts cloudflared: declare a secret delivered as {CONNECTOR_TOKEN_ENV} first')
    values = {
        'LISKOV_PLUMAGE_PAYLOAD_URL': ' '.join(part['url'] for part in identity['parts']),
        'LISKOV_PLUMAGE_PAYLOAD_SHA256': identity['sha256'],
        'LISKOV_PLUMAGE_PAYLOAD_BYTES': str(identity['bytes']),
        'LISKOV_PLUMAGE_PAYLOAD_PART_SHA256': ' '.join(part['sha256'] for part in identity['parts']),
        'LISKOV_PLUMAGE_PAYLOAD_PART_BYTES': ' '.join(str(part['bytes']) for part in identity['parts']),
    }
    variables = document.setdefault('configuration', {}).setdefault('variables', [])
    remaining = [item for item in variables if item.get('name') not in VARIABLES]
    for name in VARIABLES:
        remaining.append({'name': name, 'source': 'literal', 'value': values[name]})
    document['configuration']['variables'] = remaining
    assert_filled(document)
    return document


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--file', type=Path, default=Path('.liskov/plumage.v5.json'))
    parser.add_argument('--out', type=Path, help='write here instead of in place')
    parser.add_argument('--artifact-digest', type=digest, required=True, help='finalized runtime-image bootstrap digest')
    parser.add_argument('--payload-identity', type=Path, help='payload-identity.json from the payload workflow (single or multi-part)')
    parser.add_argument('--payload-url', help='single-part alternative to --payload-identity')
    parser.add_argument('--payload-sha256')
    parser.add_argument('--payload-bytes')
    args = parser.parse_args()
    try:
        if args.payload_identity is not None:
            if args.payload_url or args.payload_sha256 or args.payload_bytes:
                raise ValueError('pass either --payload-identity or the three single-part values')
            identity = identity_from_file(args.payload_identity)
        elif args.payload_url and args.payload_sha256 and args.payload_bytes:
            whole = {'url': https_url(args.payload_url), 'sha256': hex64(args.payload_sha256), 'bytes': size(args.payload_bytes)}
            identity = {'sha256': whole['sha256'], 'bytes': whole['bytes'], 'parts': [whole], 'connector': False}
        else:
            raise ValueError('pass --payload-identity FILE or all of --payload-url, --payload-sha256, --payload-bytes')
        document = fill(json.loads(args.file.read_text()), args.artifact_digest, identity)
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.error(str(error))
    out = args.out or args.file
    out.write_text(json.dumps(document, indent=2) + '\n')
    print(json.dumps({'written': str(out), 'artifact': args.artifact_digest, 'payload': {'sha256': identity['sha256'], 'bytes': identity['bytes'], 'parts': len(identity['parts'])}}))
    return 0


if __name__ == '__main__':
    sys.exit(main())
