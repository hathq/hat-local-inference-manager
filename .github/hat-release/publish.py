"""Refuse publication unless owner identity, protected environment and exact main CI agree."""
import hashlib
import json
import os
from pathlib import Path
import subprocess


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
    require(any(r['head_sha'] == head and r['event'] == 'push' and r['head_branch'] == 'main' and r['conclusion'] == 'success' for r in runs), 'successful main CI is required')


if __name__ == '__main__':
    identity = json.loads(Path('.github/hat-release.json').read_text())
    repo = identity['repository']
    event = json.loads(Path(os.environ['GITHUB_EVENT_PATH']).read_text())
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()
    main = subprocess.check_output(['git', 'rev-parse', 'origin/main'], text=True).strip()
    env = gh(f'repos/{repo}/environments/hat-public')
    policies = gh(f'repos/{repo}/environments/hat-public/deployment-branch-policies?per_page=100')
    runs = gh(f'repos/{repo}/actions/runs?head_sha={head}&event=push&branch=main&per_page=100')['workflow_runs']
    admit(identity, event, env, policies, head, main, runs)
    directory = Path(os.environ['HAT_RELEASE_DIRECTORY'])
    manifest = json.loads((directory / 'release.json').read_text())
    require(manifest['repository'] == repo and manifest['source'] == head)
    require(os.environ['GITHUB_REF_NAME'] == 'hat/v' + manifest['version'])
    lines = (directory / 'SHA256SUMS').read_text().splitlines()
    require(len(lines) == 3)
    names = []
    for line in lines:
        digest, name = line.split('  ')
        require(name in ['hat.package.json', 'release.json', 'hat-distribution-linux-x86_64.tar.gz'])
        names.append(name)
        require(hashlib.sha256((directory / name).read_bytes()).hexdigest() == digest)
    require(len(set(names)) == 3 and len(list(directory.iterdir())) == 4)
    # --verify-tag never creates a missing tag; an existing Release is not replaced.
    subprocess.run(['gh', 'release', 'create', os.environ['GITHUB_REF_NAME'], '--repo', repo, '--verify-tag',
                    '--title', f"HAT {manifest['package_id']} {manifest['version']}",
                    '--notes', 'Linux x86_64 build. SHA256SUMS and GitHub provenance bind this build; installation still requires the independently verified signed catalog and explicit local authorization.',
                    *[str(p) for p in sorted(directory.iterdir())]], check=True)
