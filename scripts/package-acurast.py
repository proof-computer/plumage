#!/usr/bin/env python3
"""Build a deterministic, local AArch64 plumage payload from already supplied inputs.

Does not download, upload, submit jobs or modify a Liskov application.
The packed peer.json is the fixed Liskov file. Weights are a pinned
reference, not archive bytes. No embedding is packed.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path
import struct
import tarfile
import gzip
import ipaddress

# Pinned 2026-10-28 from the Hugging Face model API and the Q4_K_M LFS object.
# Revision is the repo commit. Size and sha256 are that file's LFS oid.
MODEL_PROFILES = {
    'qwen35-4b': {
        'repository': 'unsloth/Qwen3.5-4B-GGUF',
        'revision': 'e87f176479d0855a907a41277aca2f8ee7a09523',
        'filename': 'Qwen3.5-4B-Q4_K_M.gguf',
        'bytes': 2740937888,
        'sha256': '00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4',
        'license': 'Apache-2.0',
    },
}

FORBIDDEN_PEER_KEYS = ('peer_id', 'token', 'advertisement_file', 'peers')
LAUNCHER_FIELDS = Path(__file__).resolve().parent.parent / '.liskov' / 'image-launcher-fields.json'


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def arm_elf(path):
    with path.open('rb') as stream:
        header = stream.read(20)
    if len(header) != 20 or header[:6] != b'\x7fELF\x02\x01' or struct.unpack('<H', header[18:20])[0] != 183:
        raise ValueError('binary must be a little-endian ELF64 AArch64 executable')


class VerifiedReader:
    def __init__(self, stream):
        self.stream = stream
        self.hasher = hashlib.sha256()

    def read(self, size=-1):
        data = self.stream.read(size)
        self.hasher.update(data)
        return data


def memory_floor(value):
    number = int(value)
    if not 512 <= number <= 32768:
        raise argparse.ArgumentTypeError('memory admission floor must be 512..32768 MiB')
    return number


def verify_model(path, expected_sha, profile_name=None):
    if len(expected_sha) != 64 or any(c not in '0123456789abcdef' for c in expected_sha):
        raise ValueError('model checksum must be a lowercase SHA-256 digest')
    with path.open('rb') as handle:
        if hashlib.file_digest(handle, 'sha256').hexdigest() != expected_sha:
            raise ValueError('model checksum mismatch')
    if profile_name is not None:
        profile = MODEL_PROFILES[profile_name]
        if expected_sha != profile['sha256'] or path.stat().st_size != profile['bytes']:
            raise ValueError('model does not match the selected pinned profile')
        return dict(profile)
    return None


def fixed_peer(cloud):
    """The published Liskov peer file. Identity fields stay absent."""
    liskov = {'site_listen': '127.0.0.1:8840'}
    if cloud is not None:
        liskov['cloudflared'] = cloud
    return {
        'data_dir': 'state/data',
        'site_dir': 'state/site',
        'model_url': 'http://127.0.0.1:8081/',
        'listen': '[::]:8841',
        'clock': {'real_start_ms': 0, 'sim_start_ms': 0, 'speed': 1},
        'last_day': None,
        'liskov': liskov,
    }


def require_public_peer(peer):
    present = [key for key in FORBIDDEN_PEER_KEYS if key in peer]
    if present:
        raise ValueError('peer.json must not carry ' + ', '.join(present))


def require_same_cloudflared(peer, deployment):
    left = (peer.get('liskov') or {}).get('cloudflared')
    right = deployment.get('cloudflared')
    if left != right:
        raise ValueError('deployment.json cloudflared differs from peer.json liskov.cloudflared')


def require_no_embedding(deployment):
    if 'embedding' in deployment or 'embedding_model' in deployment:
        raise ValueError('plumage payloads do not pack an embedding model')


def connector_block(args):
    if getattr(args, 'cloudflared', None) is None:
        return None
    arm_elf(args.cloudflared)
    if digest(args.cloudflared) != args.cloudflared_sha256:
        raise ValueError('cloudflared checksum mismatch')
    metrics = args.cloudflared_metrics
    host, port_text = metrics.rsplit(':', 1)
    port = int(port_text)
    if not ipaddress.ip_address(host.strip('[]')).is_loopback or not 1 <= port <= 65535 or port in (8840, 8841, 8081):
        raise ValueError('cloudflared metrics must use a distinct loopback port')
    if args.cloudflared_protocol not in ('auto', 'quic', 'http2'):
        raise ValueError('cloudflared protocol must be auto, quic or http2')
    if args.cloudflared_ca_bundle is None:
        raise ValueError('cloudflared requires the pinned CA bundle')
    binary = {'path': 'bin/cloudflared', 'bytes': args.cloudflared.stat().st_size, 'sha256': args.cloudflared_sha256}
    path = args.cloudflared_ca_bundle
    if path.stat().st_size > 16 * 1024 * 1024 or b'-----BEGIN CERTIFICATE-----' not in path.read_bytes():
        raise ValueError('cloudflared CA bundle must be bounded PEM certificates')
    ca = 'ca/ca-certificates.crt'
    ca_file = path
    # No token. The policy delivers LISKOV_PLUMAGE_CLOUDFLARED_TOKEN at launch.
    cloud = {'binary': binary, 'metrics': metrics, 'protocol': args.cloudflared_protocol, 'ca_bundle': ca}
    return cloud, ca_file


def build(args):
    arm_elf(args.plumage)
    arm_elf(args.server_dir / 'llama-server')
    if len(args.server_revision) != 40 or any(c not in '0123456789abcdef' for c in args.server_revision):
        raise ValueError('server revision must be a full lowercase commit hash')
    if len(args.model_revision) != 40 or any(c not in '0123456789abcdef' for c in args.model_revision):
        raise ValueError('model revision must be a full lowercase commit hash')
    if args.model is not None:
        verify_model(args.model, args.model_sha256, args.model_profile)
    elif args.model_profile is None:
        raise ValueError('a model file is required when no profile is selected')
    if args.model_profile is not None:
        profile = MODEL_PROFILES[args.model_profile]
        if args.model_repository != profile['repository'] or args.model_revision != profile['revision'] or args.model_sha256 != profile['sha256']:
            raise ValueError('model provenance does not match the pinned profile')
    files = {'bin/plumage': args.plumage}
    if args.model_profile is not None:
        profile = MODEL_PROFILES[args.model_profile]
        model = {'path': 'state/models/qwen35-4b-q4_k_m.gguf', 'bytes': profile['bytes'], 'sha256': profile['sha256'],
                 'url': f"https://huggingface.co/{profile['repository']}/resolve/{profile['revision']}/{profile['filename']}"}
    else:
        files['models/model.gguf'] = args.model
        model = {'path': 'models/model.gguf', 'bytes': args.model.stat().st_size, 'sha256': args.model_sha256}
    packed = connector_block(args)
    cloud = None
    extras = []
    if packed is not None:
        cloud, ca_file = packed
        files['bin/cloudflared'] = args.cloudflared
        extras.append(cloud['binary'])
        if ca_file is not None:
            files[cloud['ca_bundle']] = ca_file
            extras.append({'path': cloud['ca_bundle'], 'bytes': ca_file.stat().st_size, 'sha256': digest(ca_file)})
    inventory = []
    for path in sorted(args.server_dir.iterdir()):
        if not path.is_file():
            raise ValueError('server release directory must contain only regular files or file symlinks')
        name = 'server/' + path.name
        files[name] = path
        inventory.append({'path': name, 'bytes': path.stat().st_size, 'sha256': digest(path)})
    if len(inventory) > 64:
        raise ValueError('server release exceeds the 64-file inventory limit')
    peer = fixed_peer(cloud)
    deployment = {
        'peer_config': 'peer.json',
        'peer': {'path': 'bin/plumage', 'bytes': args.plumage.stat().st_size, 'sha256': digest(args.plumage)},
        'server_files': inventory,
        'extra_files': extras,
        'server': 'server/llama-server',
        'server_revision': args.server_revision,
        'model': model,
        'threads': args.threads,
        'min_available_memory_mib': args.min_available_memory_mib,
        'ready_seconds': 300,
        'download_seconds': 3600,
        'shutdown_seconds': 30,
        'model_port': 8081,
        'context_tokens': 4096,
        'cache_ram_mib': 1024,
        'reserved_ports': [8840, 8841],
    }
    if cloud is not None:
        deployment['cloudflared'] = cloud
    require_public_peer(peer)
    require_same_cloudflared(peer, deployment)
    require_no_embedding(deployment)
    provenance = {
        'format': 'plumage.cargo-payload.v1',
        'plumage_sha256': digest(args.plumage),
        'server_revision': args.server_revision,
        'server_build': 'operator-supplied CPU-only portable AArch64; qualification required',
        'model_repository': args.model_repository,
        'model_revision': args.model_revision,
        'model_license': args.model_license,
        'model_label': args.model_label,
        'model_sha256': args.model_sha256,
        'scope': 'local payload only; not a finalized Liskov bundle, publication, launch or device acceptance',
    }
    generated = {'peer.json': peer, 'deployment.json': deployment, 'provenance.json': provenance}
    check_launcher_fields(peer, deployment)
    pinned = {item['path']: item for item in [deployment['peer'], deployment['model'], *inventory, *extras]}
    try:
        with args.out.open('xb') as raw, gzip.GzipFile(fileobj=raw, mode='wb', filename='', mtime=0) as gz:
            with tarfile.open(fileobj=gz, mode='w|', format=tarfile.USTAR_FORMAT) as archive:
                for name in sorted(set(files) | set(generated)):
                    item = tarfile.TarInfo(name)
                    item.mode = 0o755 if name in ('bin/plumage', 'server/llama-server', 'bin/cloudflared') else 0o644
                    if name in generated:
                        data = (json.dumps(generated[name], sort_keys=True, indent=2) + '\n').encode()
                        item.size = len(data)
                        archive.addfile(item, io.BytesIO(data))
                    else:
                        item.size = pinned[name]['bytes']
                        with files[name].open('rb') as stream:
                            reader = VerifiedReader(stream)
                            archive.addfile(item, reader)
                            if stream.read(1) or reader.hasher.hexdigest() != pinned[name]['sha256']:
                                raise ValueError('input changed while packing')
    except FileExistsError:
        raise
    except BaseException:
        args.out.unlink(missing_ok=True)
        raise
    return provenance


def check_launcher_fields(config, deployment, fields_path=LAUNCHER_FIELDS):
    """Refuse keys PeerConfig or the harness Deployment cannot parse."""
    fields = json.loads(fields_path.read_text())
    problems = []

    def check(name, value, allowed):
        extra = sorted(set(value) - set(allowed))
        if extra:
            problems.append(f"{name}: {', '.join(extra)}")

    check('peer.json', config, fields['peer_json'])
    if isinstance(config.get('clock'), dict):
        check('peer.json clock', config['clock'], fields['peer_json_clock'])
    liskov = config.get('liskov')
    if isinstance(liskov, dict):
        check('peer.json liskov', liskov, fields['peer_json_liskov'])
        cloud = liskov.get('cloudflared')
        if isinstance(cloud, dict):
            check('peer.json liskov.cloudflared', cloud, fields['peer_json_cloudflared'])
            if isinstance(cloud.get('binary'), dict):
                check('peer.json liskov.cloudflared.binary', cloud['binary'], fields['deployment_input'])
    check('deployment.json', deployment, fields['deployment_json'])
    for key in ('peer', 'model'):
        if isinstance(deployment.get(key), dict):
            check(f'deployment.json {key}', deployment[key], fields['deployment_input'])
    if isinstance(deployment.get('embedding'), dict):
        check('deployment.json embedding', deployment['embedding'], fields['deployment_embedding'])
        if isinstance(deployment['embedding'].get('model'), dict):
            check('deployment.json embedding.model', deployment['embedding']['model'], fields['deployment_input'])
    if isinstance(deployment.get('cloudflared'), dict):
        check('deployment.json cloudflared', deployment['cloudflared'], fields['peer_json_cloudflared'])
        if isinstance(deployment['cloudflared'].get('binary'), dict):
            check('deployment.json cloudflared.binary', deployment['cloudflared']['binary'], fields['deployment_input'])
    for item in [*deployment.get('server_files', []), *deployment.get('extra_files', [])]:
        check('deployment.json file', item, fields['deployment_input'])
    if problems:
        raise SystemExit(f"keys unknown to the image launcher ({fields['image']}): " + '; '.join(problems))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plumage', type=Path, required=True)
    parser.add_argument('--server-dir', type=Path, required=True)
    parser.add_argument('--server-revision', required=True)
    parser.add_argument('--cloudflared', type=Path, help='supplied ARM named-tunnel connector')
    parser.add_argument('--cloudflared-sha256')
    parser.add_argument('--cloudflared-metrics', default='127.0.0.1:20241')
    parser.add_argument('--cloudflared-protocol', choices=('auto', 'quic', 'http2'), default='auto')
    parser.add_argument('--cloudflared-ca-bundle', type=Path)
    parser.add_argument('--model', type=Path, help='optional already downloaded bytes to verify; weights are referenced, not packed')
    parser.add_argument('--model-profile', choices=MODEL_PROFILES, default='qwen35-4b')
    parser.add_argument('--model-sha256', required=True)
    parser.add_argument('--model-label', required=True)
    parser.add_argument('--model-repository', required=True)
    parser.add_argument('--model-revision', required=True)
    parser.add_argument('--model-license', required=True)
    parser.add_argument('--threads', type=int, choices=range(1, 9), default=4)
    parser.add_argument('--min-available-memory-mib', type=memory_floor, required=True, metavar='MIB')
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    try:
        result = build(args)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    print(json.dumps({'payload': str(args.out), 'sha256': digest(args.out), 'provenance': result}, sort_keys=True))


if __name__ == '__main__':
    main()
