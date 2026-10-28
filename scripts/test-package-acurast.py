#!/usr/bin/env python3
"""Offline payload tests. ARM headers are fixtures, not runnable device proof."""
import argparse
import hashlib
import importlib.util
import json
import os
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('package', Path(__file__).with_name('package-acurast.py'))
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)
spec = importlib.util.spec_from_file_location('release', Path(__file__).with_name('fill-v5-release.py'))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class PackageTests(unittest.TestCase):
    def fixture(self, root):
        header = b'\x7fELF\x02\x01' + bytes(12) + b'\xb7\x00'
        (root / 'plumage').write_bytes(header)
        (root / 'server').mkdir()
        (root / 'server' / 'llama-server').write_bytes(header)
        (root / 'server' / 'libllama.so').write_bytes(b'library')
        (root / 'model').write_bytes(b'GGUF-substitute')
        return argparse.Namespace(
            plumage=root / 'plumage', server_dir=root / 'server', server_revision='a' * 40,
            model=root / 'model', model_profile=None, model_sha256=package.digest(root / 'model'),
            model_repository='fixture/model', model_revision='b' * 40, model_license='fixture',
            model_label='offline', threads=1, min_available_memory_mib=512, out=root / 'one.tar.gz',
            cloudflared=None,
        )

    def test_fixture_pack_is_deterministic_and_carries_the_fixed_peer(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            args = self.fixture(root)
            package.build(args)
            first = package.digest(args.out)
            args.out = root / 'two.tar.gz'
            package.build(args)
            self.assertEqual(first, package.digest(args.out))
            with tarfile.open(args.out) as archive:
                names = set(archive.getnames())
                self.assertTrue({'bin/plumage', 'server/llama-server', 'deployment.json', 'peer.json'} <= names)
                self.assertEqual(archive.getmember('bin/plumage').mode, 0o755)
                peer = json.load(archive.extractfile('peer.json'))
                deployment = json.load(archive.extractfile('deployment.json'))
                for item in deployment['server_files']:
                    data = archive.extractfile(item['path']).read()
                    self.assertEqual(item['sha256'], hashlib.sha256(data).hexdigest())
                    self.assertEqual(item['bytes'], len(data))
            self.assertEqual(peer['listen'], '[::]:8841')
            self.assertEqual(peer['model_url'], 'http://127.0.0.1:8081/')
            self.assertEqual(peer['data_dir'], 'state/data')
            self.assertEqual(peer['site_dir'], 'state/site')
            self.assertEqual(peer['clock'], {'real_start_ms': 0, 'sim_start_ms': 0, 'speed': 1})
            self.assertIsNone(peer['last_day'])
            self.assertEqual(peer['liskov'], {'site_listen': '127.0.0.1:8840'})
            for key in package.FORBIDDEN_PEER_KEYS:
                self.assertNotIn(key, peer)
            self.assertEqual(deployment['peer_config'], 'peer.json')
            self.assertEqual(deployment['peer']['path'], 'bin/plumage')
            self.assertEqual(deployment['model_port'], 8081)
            self.assertEqual(deployment['context_tokens'], 4096)
            self.assertEqual(deployment['cache_ram_mib'], 1024)
            self.assertEqual(deployment['reserved_ports'], [8840, 8841])
            self.assertNotIn('embedding', deployment)
            self.assertNotIn('cloudflared', deployment)
            self.assertNotIn('token', json.dumps(peer))
            with self.assertRaises(FileExistsError):
                package.build(args)
            self.assertEqual(first, package.digest(args.out))

    def test_unknown_launcher_key_is_refused(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            args = self.fixture(root)
            original = package.fixed_peer

            def bad(cloud):
                peer = original(cloud)
                peer['render_tokens'] = 4096
                return peer

            package.fixed_peer = bad
            try:
                with self.assertRaises(SystemExit) as refused:
                    package.build(args)
                self.assertIn('render_tokens', str(refused.exception))
                self.assertFalse(args.out.exists())
            finally:
                package.fixed_peer = original

    def test_cloudflared_blocks_must_match_and_carry_no_secret(self):
        with self.assertRaisesRegex(ValueError, 'differs'):
            package.require_same_cloudflared(
                {'liskov': {'cloudflared': {'protocol': 'auto'}}},
                {'cloudflared': {'protocol': 'quic'}},
            )
        peer = package.fixed_peer(None)
        peer['token'] = 'must-never-ship'
        with self.assertRaisesRegex(ValueError, 'token'):
            package.require_public_peer(peer)
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            args = self.fixture(root)
            args.cloudflared = root / 'plumage'
            args.cloudflared_sha256 = package.digest(args.cloudflared)
            args.cloudflared_metrics = '127.0.0.1:20241'
            args.cloudflared_protocol = 'quic'
            args.cloudflared_ca_bundle = root / 'ca.pem'
            args.cloudflared_ca_bundle.write_text('-----BEGIN CERTIFICATE-----\nfixture\n-----END CERTIFICATE-----\n')
            package.build(args)
            with tarfile.open(args.out) as archive:
                packed_peer = json.load(archive.extractfile('peer.json'))
                deployment = json.load(archive.extractfile('deployment.json'))
                self.assertEqual(packed_peer['liskov']['cloudflared'], deployment['cloudflared'])
                self.assertNotIn('token', json.dumps(packed_peer))
                self.assertEqual(deployment['cloudflared']['binary']['sha256'], args.cloudflared_sha256)
                self.assertEqual(deployment['cloudflared']['protocol'], 'quic')
                self.assertEqual(archive.getmember('bin/cloudflared').mode, 0o755)
                self.assertEqual(len(deployment['extra_files']), 2)
            args.out = root / 'bad.tar.gz'
            args.cloudflared_metrics = '0.0.0.0:20241'
            with self.assertRaisesRegex(ValueError, 'loopback'):
                package.build(args)
            self.assertFalse(args.out.exists())

    def test_pinned_profile_references_weights_and_packs_no_gguf(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            args = self.fixture(root)
            args.model_profile = 'offline-profile'
            package.MODEL_PROFILES['offline-profile'] = {
                'repository': args.model_repository, 'revision': args.model_revision,
                'filename': 'fixture.gguf', 'sha256': args.model_sha256,
                'bytes': args.model.stat().st_size,
            }
            try:
                package.build(args)
                with tarfile.open(args.out) as archive:
                    self.assertFalse(any(name.endswith('.gguf') for name in archive.getnames()))
                    deployment = json.load(archive.extractfile('deployment.json'))
                    self.assertIn(args.model_revision, deployment['model']['url'])
                    self.assertEqual(deployment['model']['sha256'], args.model_sha256)
                    self.assertNotIn('embedding', deployment)
            finally:
                del package.MODEL_PROFILES['offline-profile']

    def test_wrong_target_and_corrupt_model_are_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            args = self.fixture(root)
            args.model_sha256 = '0' * 64
            with self.assertRaisesRegex(ValueError, 'checksum'):
                package.build(args)
            self.assertFalse(args.out.exists())
            (root / 'plumage').write_bytes(b'host executable')
            with self.assertRaisesRegex(ValueError, 'AArch64'):
                package.build(args)

    def test_workflow_pins_the_4b_profile_without_embeddings_or_secrets(self):
        workflow = (Path(__file__).resolve().parents[1] / '.github' / 'workflows' / 'plumage-payload.yml').read_text()
        profile = package.MODEL_PROFILES['qwen35-4b']
        self.assertEqual(profile['bytes'], 2740937888)
        self.assertIn(profile['sha256'], workflow)
        self.assertIn(profile['revision'], workflow)
        self.assertIn(profile['repository'], workflow)
        self.assertIn('--model-profile qwen35-4b', workflow)
        self.assertIn('LISKOV_PLUMAGE_BUILD_COMMIT', workflow)
        self.assertIn('plumage.payload-identity.v1', workflow)
        self.assertIn('default: "4096"', workflow)
        self.assertIn('default: true', workflow)
        self.assertNotIn('--with-embeddings', workflow)
        self.assertNotIn('secrets.', workflow)
        self.assertNotIn('peer-id:', workflow)
        setup = workflow.split('          connector=()\n', 1)[1].split('          python3 scripts/package-acurast.py', 1)[0]
        for enabled in (False, True):
            env = dict(os.environ, CLOUDFLARED=str(enabled).lower(), CLOUDFLARED_SHA256='abc', CLOUDFLARED_PROTOCOL='http2')
            flag = subprocess.check_output(
                ['bash', '-eu', '-c', 'connector=()\n' + setup + '\nprintf "%s\\n" "${connector[*]}"'],
                env=env, text=True)
            if enabled:
                self.assertIn('--cloudflared-protocol', flag)
                self.assertIn('http2', flag)
            else:
                self.assertEqual(flag.strip(), '')


class ReleaseBindingTests(unittest.TestCase):
    def identity(self, root, connector):
        path = root / 'payload-identity.json'
        path.write_text(json.dumps({
            'format': 'plumage.payload-identity.v1', 'sha256': 'a' * 64, 'bytes': 7, 'cloudflared': connector,
            'parts': [
                {'url': 'https://gateway.example/ipfs/one', 'sha256': 'b' * 64, 'bytes': 3},
                {'url': 'https://gateway.example/ipfs/two', 'sha256': 'c' * 64, 'bytes': 4},
            ],
        }))
        return release.identity_from_file(path)

    def test_fill_writes_five_variables_and_refuses_the_unfilled_draft(self):
        draft_path = Path(__file__).resolve().parents[1] / '.liskov' / 'plumage.v5.json'
        draft = json.loads(draft_path.read_text())
        self.assertEqual(draft['release']['artifact']['digest'], release.ZERO_DIGEST)
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            identity = self.identity(root, {'token_env': 'LISKOV_PLUMAGE_CLOUDFLARED_TOKEN'})
            with self.assertRaisesRegex(ValueError, 'all-zero'):
                release.fill(json.loads(json.dumps(draft)), release.ZERO_DIGEST, identity)
            filled = release.fill(json.loads(json.dumps(draft)), 'sha256:' + 'f' * 64, identity)
            values = {item['name']: item['value'] for item in filled['configuration']['variables']}
            self.assertEqual(values['LISKOV_PLUMAGE_HOME'], '/app')
            self.assertEqual(values['LISKOV_PLUMAGE_PAYLOAD_URL'], 'https://gateway.example/ipfs/one https://gateway.example/ipfs/two')
            self.assertEqual(values['LISKOV_PLUMAGE_PAYLOAD_SHA256'], 'a' * 64)
            self.assertEqual(values['LISKOV_PLUMAGE_PAYLOAD_BYTES'], '7')
            self.assertEqual(values['LISKOV_PLUMAGE_PAYLOAD_PART_SHA256'], f"{'b' * 64} {'c' * 64}")
            self.assertEqual(values['LISKOV_PLUMAGE_PAYLOAD_PART_BYTES'], '3 4')
            self.assertEqual(filled['configuration']['secrets'], draft['configuration']['secrets'])
            bare = {'schema': 'proof.liskov.application-manifest', 'schemaVersion': 5, 'release': {'mode': 'pinned'},
                    'configuration': {'variables': [], 'secrets': []}}
            with self.assertRaisesRegex(ValueError, 'declare a secret'):
                release.fill(json.loads(json.dumps(bare)), 'sha256:' + 'f' * 64, identity)
            with self.assertRaisesRegex(ValueError, 'unknown connector'):
                self.identity(root, {'token_env': 'TUNNEL_TOKEN'})
            self.identity(root, {'token_env': 'LISKOV_PLUMAGE_CLOUDFLARED_TOKEN'})
            copy = root / 'plumage.v5.json'
            copy.write_text(draft_path.read_text())
            before = copy.read_text()
            script = Path(__file__).resolve().with_name('fill-v5-release.py')
            refused = subprocess.run(
                [sys.executable, str(script), '--file', str(copy), '--artifact-digest', release.ZERO_DIGEST,
                 '--payload-identity', str(root / 'payload-identity.json')],
                capture_output=True, text=True)
            self.assertNotEqual(refused.returncode, 0)
            self.assertEqual(copy.read_text(), before)
            out = root / 'filled.json'
            bound = subprocess.run(
                [sys.executable, str(script), '--file', str(copy), '--out', str(out),
                 '--artifact-digest', 'sha256:' + 'ab' * 32, '--payload-identity', str(root / 'payload-identity.json')],
                capture_output=True, text=True)
            self.assertEqual(bound.returncode, 0, bound.stderr)
            written = json.loads(out.read_text())
            written_values = {item['name']: item['value'] for item in written['configuration']['variables']}
            self.assertEqual(written_values['LISKOV_PLUMAGE_PAYLOAD_BYTES'], '7')
            self.assertEqual(len(release.VARIABLES), 5)


class BuildDocumentTests(unittest.TestCase):
    root = Path(__file__).resolve().parent.parent

    def document(self, name):
        return json.loads((self.root / '.liskov' / name).read_text())

    def test_build_document_is_a_source_release_with_the_policy_runtime(self):
        build = self.document('plumage.source.v5.json')
        published = self.document('plumage.v5.json')
        self.assertEqual(build['release'], {'mode': 'source'})
        self.assertEqual(published['release']['mode'], 'pinned')
        self.assertEqual(published['release']['artifact']['digest'], release.ZERO_DIGEST)
        self.assertEqual(build['applicationId'], published['applicationId'])
        self.assertEqual(build['applicationId'], 'plumage')
        self.assertEqual(build['runtime'], published['runtime'])
        self.assertEqual(build['runtime']['entrypoint'], {'executable': '/usr/local/bin/plumage', 'args': ['launch']})
        self.assertEqual(published['deployment']['jobs'], 2)
        self.assertEqual(published['deployment']['schedule']['duration'], '2h')
        self.assertEqual(published['deployment']['spend'], {
            'unit': 'service_credit_micros', 'perJob': '1200000',
            'rate': {'amount': '6000000', 'window': '1d'},
        })
        self.assertEqual(build['deployment']['spend']['perJob'], '1200000')
        self.assertEqual(published['deployment']['placement']['minimums'], {'memory': '8GiB', 'storage': '8GiB'})
        self.assertNotIn('processorSelection', published['deployment']['placement'])
        self.assertTrue(published['observability']['logs']['enabled'])
        self.assertEqual(published['access']['ssh']['provider']['kind'], 'liskov_managed')
        names = [item['name'] for item in published['configuration']['variables']]
        self.assertEqual(names, ['LISKOV_PLUMAGE_HOME', *release.VARIABLES])
        for item in published['configuration']['variables'][1:]:
            self.assertEqual(item['value'], '')

    def test_image_workflow_uploads_the_build_document(self):
        workflow = (self.root / '.github' / 'workflows' / 'plumage-runtime-image.yml').read_text()
        self.assertIn('cargo-runtime-image.yml@v2', workflow)
        self.assertIn('application-id: plumage', workflow)
        self.assertIn('manifest-path: .liskov/plumage.source.v5.json', workflow)
        self.assertIn('binary-name: plumage', workflow)
        self.assertIn('install-path: /usr/local/bin/plumage', workflow)
        self.assertIn('base-image-sha256: 0639e88db6b46cef6091acafe35dfb1b59c5e354463d969f1a9509451d118377', workflow)
        self.assertNotIn('secrets.', workflow)


if __name__ == '__main__':
    unittest.main()
