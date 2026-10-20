"""Refuse publication unless owner identity, protected environment and exact main CI agree."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile


def gh(path):
    return json.loads(subprocess.check_output(['gh', 'api', path], text=True))


def require(condition, message='release boundary rejected'):
    if not condition:
        raise ValueError(message)


def admit(identity, event, environment, policies, head, main, runs):
    require(event['repository']['id'] == identity['github_id'])
    require(event['repository']['full_name'] == identity['repository'])
    require(event['repository']['private'] is False)
    require(head == main, 'tag must resolve the reviewed current main')
    review = next((r for r in environment['protection_rules'] if r['type'] == 'required_reviewers'), {})
    require(review.get('reviewers'), 'native reviewer protection is required')
    require(environment['deployment_branch_policy']['custom_branch_policies'] is True)
    require(any(p['type'] == 'tag' and p['name'] == 'hat/v*' for p in policies['branch_policies']))
    paths = identity.get('required_workflows', [])
    require(paths and len(paths) <= 16 and len(set(paths)) == len(paths), 'explicit owner workflows required')
    for path in paths:
        matching = [r for r in runs if r['head_sha'] == head and r['event'] == 'push'
                    and r['head_branch'] == 'main' and r.get('path') == path]
        require(matching, 'required main workflow missing')
        key = lambda r: (r.get('run_number', 0), r.get('id', 0))
        latest_key = max(map(key, matching))
        latest = [r for r in matching if key(r) == latest_key]
        require(all(r.get('status') == 'completed' and r['conclusion'] == 'success' for r in latest),
                'latest required main workflow must succeed')


def publish_once(repo, tag, head, directory):
    paths = sorted(directory.iterdir())
    existing = subprocess.run(['gh', 'api', f'repos/{repo}/releases/tags/{tag}'], capture_output=True, text=True)
    if existing.returncode:
        require('(HTTP 404)' in existing.stderr, 'release state unknown; no write')
        require(os.environ.get('GITHUB_RUN_ATTEMPT') == '1', 'no blind write retry')
        subprocess.run(['gh', 'release', 'create', tag, '--repo', repo, '--verify-tag', '--target', head,
                        '--title', tag, '--notes', 'Owner-built Linux HAT. Installation still requires signed catalog trust.',
                        *[str(p) for p in paths]], check=True)
    else:
        release = json.loads(existing.stdout)
        require(not release.get('draft') and release.get('tag_name') == tag, 'release identity mismatch')
    with tempfile.TemporaryDirectory() as temporary:
        for path in paths:
            subprocess.run(['gh', 'release', 'download', tag, '--repo', repo, '--pattern', path.name,
                            '--dir', temporary], check=True)
            observed = Path(temporary) / path.name
            require(observed.stat().st_size == path.stat().st_size, 'released size mismatch')
            require(hashlib.sha256(observed.read_bytes()).digest() == hashlib.sha256(path.read_bytes()).digest(),
                    'released bytes mismatch')


if __name__ == '__main__':
    identity = json.loads(Path('.github/hat-release.json').read_text())
    repo = identity['repository']
    event = json.loads(Path(os.environ['GITHUB_EVENT_PATH']).read_text())
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()
    main = subprocess.check_output(['git', 'rev-parse', 'origin/main'], text=True).strip()
    env = gh(f'repos/{repo}/environments/hat-public')
    policies = gh(f'repos/{repo}/environments/hat-public/deployment-branch-policies?per_page=100')
    response = gh(f'repos/{repo}/actions/runs?head_sha={head}&event=push&branch=main&per_page=100')
    runs = response['workflow_runs']
    require(response['total_count'] == len(runs) and len(runs) < 100, 'partial main CI observation')
    require(policies['total_count'] == len(policies['branch_policies']), 'partial environment policy')
    admit(identity, event, env, policies, head, main, runs)
    directory = Path(os.environ['HAT_RELEASE_DIRECTORY'])
    manifest = json.loads((directory / 'release.json').read_text())
    descriptor = json.loads(Path('hat.package.json').read_text())
    require(manifest['repository'] == repo and manifest['source'] == head)
    require(manifest['version'] == descriptor['version'] and manifest['package_id'] == identity['package_id'])
    require((directory / 'hat.package.json').read_bytes() == Path('hat.package.json').read_bytes())
    tag = os.environ['GITHUB_REF_NAME']
    require(tag == 'hat/v' + manifest['version'])
    lines = (directory / 'SHA256SUMS').read_text().splitlines()
    require(len(lines) == 3)
    names = []
    for line in lines:
        digest, name = line.split('  ')
        require(name in ['hat.package.json', 'release.json', 'hat-distribution-linux-x86_64.tar.gz'])
        names.append(name)
        require(hashlib.sha256((directory / name).read_bytes()).hexdigest() == digest)
    require(len(set(names)) == 3 and len(list(directory.iterdir())) == 4)
    publish_once(repo, tag, head, directory)
