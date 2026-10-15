import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


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
        identity = {'repository':'example/hat-example', 'github_id':42}
        event = {'repository':{'id':42, 'full_name':identity['repository'], 'private':False}}
        environment = {'protection_rules':[{'type':'required_reviewers', 'reviewers':[{'id':1}]}], 'deployment_branch_policy':{'custom_branch_policies':True}}
        policy = {'branch_policies':[{'type':'tag','name':'hat/v*'}]}
        runs = [{'head_sha':'a'*40,'event':'push','head_branch':'main','conclusion':'success'}]
        publish.admit(identity,event,environment,policy,'a'*40,'a'*40,runs)
        for patch in [{'private':True},{'id':43},{'full_name':'foreign/hat-example'}]:
            changed=copy.deepcopy(event);changed['repository'].update(patch)
            with self.assertRaises(ValueError): publish.admit(identity,changed,environment,policy,'a'*40,'a'*40,runs)
        for changed in [{**environment,'protection_rules':[]},{**environment,'deployment_branch_policy':{'custom_branch_policies':False}}]:
            with self.assertRaises(ValueError): publish.admit(identity,event,changed,policy,'a'*40,'a'*40,runs)
        for head, main, outcomes in [('b'*40,'a'*40,runs),('a'*40,'a'*40,[]),('a'*40,'a'*40,[{**runs[0],'conclusion':'failure'}])]:
            with self.assertRaises(ValueError): publish.admit(identity,event,environment,policy,head,main,outcomes)


if __name__ == '__main__': unittest.main()
