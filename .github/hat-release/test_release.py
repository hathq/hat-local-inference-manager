import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import subprocess
import os


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


prepare, publish = load('prepare'), load('publish')


class ReleaseBoundaries(unittest.TestCase):
    def test_hat_version_and_identity_not_cargo_version(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data = dict(schema='hathq://hat/package/v2', version='2.3.4', repository_id='hat-example', package_id='hat/example', manifest={'version':'2.3.4'})
            (root/'hat.package.json').write_text(json.dumps(data))
            identity = {'repository':'example/hat-example', 'package_id':'hat/example'}
            self.assertEqual(prepare.descriptor(root, identity, 'hat/v2.3.4')[0]['version'], '2.3.4')
            for tag in ['v2.3.4', 'hat/v0.1.0', 'hat/v2.3.4-beta.1']:
                with self.assertRaises(ValueError): prepare.descriptor(root, identity, tag)
            with self.assertRaises(ValueError): prepare.descriptor(root, {**identity, 'package_id':'hat/foreign'})

    def test_native_review_and_exact_public_owner_required(self):
        identity = {'repository':'example/hat-example', 'github_id':42, 'required_workflows':['.github/workflows/ci.yml']}
        event = {'repository':{'id':42, 'full_name':identity['repository'], 'private':False}}
        environment = {'protection_rules':[{'type':'required_reviewers', 'reviewers':[{'id':1}]}], 'deployment_branch_policy':{'custom_branch_policies':True}}
        policy = {'branch_policies':[{'type':'tag','name':'hat/v*'}]}
        runs = [{'head_sha':'a'*40,'event':'push','head_branch':'main','conclusion':'success','status':'completed','path':'.github/workflows/ci.yml','run_number':2}]
        publish.admit(identity,event,environment,policy,'a'*40,'a'*40,runs)
        for patch in [{'private':True},{'id':43},{'full_name':'foreign/hat-example'}]:
            changed=copy.deepcopy(event);changed['repository'].update(patch)
            with self.assertRaises(ValueError): publish.admit(identity,changed,environment,policy,'a'*40,'a'*40,runs)
        for changed in [{**environment,'protection_rules':[]},{**environment,'deployment_branch_policy':{'custom_branch_policies':False}}]:
            with self.assertRaises(ValueError): publish.admit(identity,event,changed,policy,'a'*40,'a'*40,runs)
        publish.admit(identity,event,environment,policy,'a'*40,'a'*40,[{**runs[0], 'conclusion':'failure','run_number':1},*runs])
        for head, main, outcomes in [('b'*40,'a'*40,runs),('a'*40,'a'*40,[]),('a'*40,'a'*40,[{**runs[0],'conclusion':'failure'}]),('a'*40,'a'*40,[{**runs[0],'conclusion':'failure','run_number':3}]),('a'*40,'a'*40,runs+[{**runs[0],'conclusion':None}])]:
            with self.assertRaises(ValueError): publish.admit(identity,event,environment,policy,head,main,outcomes)

    def test_unknown_release_or_uncertain_retry_never_writes(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(publish.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1, '', 'HTTP 500')) as run:
                with self.assertRaises(ValueError): publish.publish_once('example/repo', 'hat/v1.0.0', 'a'*40, Path(directory))
                self.assertEqual(run.call_count, 1)
            with patch.dict(os.environ, {'GITHUB_RUN_ATTEMPT':'2'}):
                with patch.object(publish.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1, '', '(HTTP 404)')) as run:
                    with self.assertRaises(ValueError): publish.publish_once('example/repo', 'hat/v1.0.0', 'a'*40, Path(directory))
                    self.assertEqual(run.call_count, 1)

    def test_existing_release_is_read_back_without_creation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root/'SHA256SUMS').write_bytes(b'expected')
            def call(argv, **kwargs):
                if argv[1] == 'api': return subprocess.CompletedProcess(argv, 0, '{"tag_name":"hat/v1.0.0","draft":false}', '')
                self.assertEqual(argv[1:3], ['release','download'])
                target = Path(argv[argv.index('--dir')+1]) / argv[argv.index('--pattern')+1]
                target.write_bytes(b'expected'); return subprocess.CompletedProcess(argv, 0)
            with patch.object(publish.subprocess, 'run', side_effect=call): publish.publish_once('example/repo','hat/v1.0.0','a'*40,root)

    def test_created_release_has_exact_post_state_and_tampering_refuses(self):
        for observed in [b'expected', b'changed!']:
            with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {'GITHUB_RUN_ATTEMPT':'1'}):
                root = Path(directory); (root/'SHA256SUMS').write_bytes(b'expected'); created=[]
                def call(argv, **kwargs):
                    if argv[1]=='api': return subprocess.CompletedProcess(argv,1,'','(HTTP 404)')
                    if argv[2]=='create': created.append(argv); return subprocess.CompletedProcess(argv,0)
                    target=Path(argv[argv.index('--dir')+1])/argv[argv.index('--pattern')+1]
                    target.write_bytes(observed); return subprocess.CompletedProcess(argv,0)
                with patch.object(publish.subprocess,'run',side_effect=call):
                    if observed==b'expected': publish.publish_once('example/repo','hat/v1.0.0','a'*40,root)
                    else:
                        with self.assertRaises(ValueError): publish.publish_once('example/repo','hat/v1.0.0','a'*40,root)
                self.assertEqual(len(created),1)


if __name__ == '__main__': unittest.main()
