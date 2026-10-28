#!/usr/bin/env python3
"""Build the pinned portable CPU-only ARM server off-device; no downloads.

The ISA is ARMv8.2-A with dotprod and fp16: the widest set the pinned
phones report (2026-10-11 /proc/cpuinfo readback). i8mm is left out
because the Tensor G2 phone lacks it and would stop with SIGILL.
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess

REVISION = 'd2e54583c7452353eb35d40431281f6ee984332f'
FLAGS = {
    'CMAKE_BUILD_TYPE': 'Release', 'CMAKE_SYSTEM_NAME': 'Linux',
    'CMAKE_SYSTEM_PROCESSOR': 'aarch64', 'CMAKE_BUILD_RPATH_USE_ORIGIN': 'ON',
    'GGML_NATIVE': 'OFF', 'GGML_CPU_ARM_ARCH': 'armv8.2-a+dotprod+fp16',
    'GGML_VULKAN': 'OFF', 'GGML_CUDA': 'OFF', 'GGML_BACKEND_DL': 'OFF',
    'GGML_OPENMP': 'OFF', 'GGML_BLAS': 'OFF', 'LLAMA_OPENSSL': 'OFF',
    'LLAMA_BUILD_UI': 'OFF', 'LLAMA_USE_PREBUILT_UI': 'OFF',
    'LLAMA_BUILD_TESTS': 'OFF', 'LLAMA_BUILD_EXAMPLES': 'OFF', 'LLAMA_BUILD_APP': 'OFF',
    'LLAMA_BUILD_SERVER': 'ON', 'GGML_CCACHE': 'OFF',
}


def build(args):
    source = args.source.resolve()
    head = subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip()
    dirty = subprocess.check_output(['git', '-C', str(source), 'status', '--porcelain'], text=True)
    if head != REVISION or dirty:
        raise ValueError('source must be a clean checkout of the pinned llama.cpp revision')
    if args.build_dir.exists() or args.release_dir.exists():
        raise ValueError('build and release directories must be new')
    flags = dict(FLAGS)
    flags.update(CMAKE_C_COMPILER=args.cc, CMAKE_CXX_COMPILER=args.cxx)
    subprocess.run(['cmake', '-S', str(source), '-B', str(args.build_dir),
                    *[f'-D{key}={value}' for key, value in sorted(flags.items())]], check=True)
    subprocess.run(['cmake', '--build', str(args.build_dir), '--target', 'llama-server',
                    '--parallel', str(args.jobs)], check=True)
    args.release_dir.mkdir()
    for path in sorted((args.build_dir/'bin').iterdir()):
        if path.name == 'llama-server' or '.so' in path.name:
            shutil.copyfile(path, args.release_dir/path.name)
    (args.release_dir/'llama-server').chmod(0o755)
    license_path = source/'LICENSE'
    if not license_path.exists():
        license_path = source/'LICENSE.md'
    shutil.copyfile(license_path, args.release_dir/'LICENSE.llama.cpp')
    shutil.copyfile(args.build_dir/'CMakeCache.txt', args.release_dir/'CMakeCache.txt')
    provenance = {'source_repository': 'https://github.com/ggml-org/llama.cpp',
                  'source_revision': head, 'cmake_flags': flags,
                  'compiler': subprocess.check_output([args.cxx, '--version'], text=True).splitlines()[0],
                  'scope': 'portable CPU-only AArch64 build (armv8.2-a+dotprod+fp16); processor/model acceptance still required'}
    (args.release_dir/'build-provenance.json').write_text(json.dumps(provenance, sort_keys=True, indent=2)+'\n')
    return provenance


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--build-dir', type=Path, required=True)
    parser.add_argument('--release-dir', type=Path, required=True)
    parser.add_argument('--cc', default='cc')
    parser.add_argument('--cxx', default='c++')
    parser.add_argument('--jobs', type=int, choices=range(1,33), default=8)
    args = parser.parse_args()
    try:
        result = build(args)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.error(str(error))
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    main()
