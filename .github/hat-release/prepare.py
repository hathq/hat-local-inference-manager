"""Public-owner HAT release assembly. No signing, deployment or installation."""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile


def descriptor(root, identity, tag=None):
    if (root / 'hat.package.json').is_symlink():
        raise ValueError('regular descriptor required')
    raw = (root / 'hat.package.json').read_bytes()
    if len(raw) > 1024 * 1024:
        raise ValueError('descriptor limit')
    value = json.loads(raw)
    version = value.get('version', '')
    if not re.fullmatch(r'\d+\.\d+\.\d+', version):
        raise ValueError('stable HAT version required')
    if (value.get('schema') != 'hathq://hat/package/v2' or value.get('repository_id') != identity['repository'].split('/')[1]
            or value.get('package_id') != identity['package_id'] or value.get('manifest', {}).get('version') != version):
        raise ValueError('HAT identity mismatch')
    if tag is not None and tag != 'hat/v' + version:
        raise ValueError('tag must match HAT descriptor, not Cargo version')
    return value, raw


def assemble(root, output, binary, identity, tag=None):
    value, raw = descriptor(root, identity, tag)
    if binary.is_symlink() or not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError('regular executable required')
    if binary.stat().st_size > 64 * 1024 * 1024:
        raise ValueError('worker binary limit')
    source = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip()
    if not re.fullmatch(r'[a-f0-9]{40}', source):
        raise ValueError('immutable source required')
    if output.exists():
        raise ValueError('choose a fresh output directory')
    output.mkdir(parents=True)
    (output / 'hat.package.json').write_bytes(raw)
    archive = subprocess.check_output(['git', 'archive', '--format=tar', '--prefix=source/', source], cwd=root)
    name = 'hat-distribution-linux-x86_64.tar.gz'
    with (output / name).open('wb') as file, gzip.GzipFile(fileobj=file, filename='', mode='wb', mtime=0) as zipped, tarfile.open(fileobj=zipped, mode='w') as bundle:
        with tarfile.open(fileobj=io.BytesIO(archive), mode='r:') as original:
            for entry in original:
                if entry.issym() or entry.islnk() or entry.isdev() or '..' in Path(entry.name).parts:
                    raise ValueError('unsupported source entry')
                if entry.isfile() or entry.isdir():
                    bundle.addfile(entry, original.extractfile(entry) if entry.isfile() else None)
        info = tarfile.TarInfo('bin/' + identity['binary'])
        info.size = binary.stat().st_size
        info.mode = 0o755
        with binary.open('rb') as stream:
            bundle.addfile(info, stream)
    manifest = dict(schema='hathq://hat/release-build/v1', repository=identity['repository'], github_repository_id=identity['github_id'],
                    source=source, version=value['version'], package_id=value['package_id'], platform='linux-x86_64',
                    installation_authority=False, catalog_signature='not-supplied',
                    files={p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in output.iterdir()})
    (output / 'release.json').write_text(json.dumps(manifest, indent=2) + '\n')
    sums = ''.join(f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n' for p in sorted(output.iterdir()))
    (output / 'SHA256SUMS').write_text(sums)
    return manifest


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('output', type=Path)
    parser.add_argument('binary', type=Path)
    args = parser.parse_args()
    root = Path.cwd()
    identity = json.loads((root / '.github/hat-release.json').read_text())
    tag = os.environ.get('GITHUB_REF_NAME') if os.environ.get('GITHUB_REF_TYPE') == 'tag' else None
    print(json.dumps(assemble(root, args.output, args.binary, identity, tag)))
