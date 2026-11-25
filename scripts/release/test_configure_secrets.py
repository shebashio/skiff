import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("configure_secrets", Path(__file__).with_name("configure-secrets.py"))
configure_secrets = importlib.util.module_from_spec(spec)
spec.loader.exec_module(configure_secrets)
problems = configure_secrets.release_protection_problems

REPO = "owner/repo"
TAG_RULESET = {
    "target": "tag",
    "enforcement": "active",
    "conditions": {"ref_name": {"include": ["refs/tags/v*"], "exclude": []}},
    "rules": [{"type": "creation"}, {"type": "update"}, {"type": "deletion"}],
}


def github(environment=None, policies=None, rulesets=None):
    """A fake GitHub API: a protected release setup, with any part replaced."""
    environment = environment if environment is not None else {
        "deployment_branch_policy": {"custom_branch_policies": True, "protected_branches": False},
        "protection_rules": [
            {"type": "branch_policy"},
            {"type": "required_reviewers", "prevent_self_review": True, "reviewers": [{"type": "User"}]},
        ],
    }
    policies = policies if policies is not None else {"total_count": 1, "branch_policies": [{"type": "tag", "name": "v*"}]}
    rulesets = rulesets if rulesets is not None else [TAG_RULESET]
    responses = {
        f"repos/{REPO}/environments/release": environment,
        f"repos/{REPO}/environments/release/deployment-branch-policies?per_page=100": policies,
        f"repos/{REPO}/rulesets?includes_parents=true&per_page=100": [{"id": i, "target": r["target"]} for i, r in enumerate(rulesets)],
    }
    for i, ruleset in enumerate(rulesets):
        responses[f"repos/{REPO}/rulesets/{i}?includes_parents=true"] = ruleset
    return lambda path: responses.get(path)


class ReleaseProtectionTests(unittest.TestCase):
    def test_protected_setup_passes(self):
        self.assertEqual(problems(REPO, github()), [])

    def test_missing_environment_stops_everything(self):
        self.assertEqual(len(problems(REPO, lambda path: None)), 1)

    def test_environment_without_reviewers_is_refused(self):
        env = {"deployment_branch_policy": {"custom_branch_policies": True}, "protection_rules": [{"type": "branch_policy"}]}
        self.assertTrue(any("reviewer" in p for p in problems(REPO, github(environment=env))))

    def test_self_review_is_refused(self):
        env = {"deployment_branch_policy": {"custom_branch_policies": True},
               "protection_rules": [{"type": "required_reviewers", "prevent_self_review": False, "reviewers": [{"type": "User"}]}]}
        self.assertTrue(any("self-review" in p for p in problems(REPO, github(environment=env))))

    def test_branch_access_is_refused(self):
        policies = {"total_count": 2, "branch_policies": [{"type": "tag", "name": "v*"}, {"type": "branch", "name": "main"}]}
        self.assertTrue(any("only the v* tag" in p for p in problems(REPO, github(policies=policies))))

    def test_tag_rulesets_must_cover_v_tags_and_restrict_all_changes(self):
        weak = [
            [],
            [dict(TAG_RULESET, enforcement="evaluate")],
            [dict(TAG_RULESET, target="branch")],
            [dict(TAG_RULESET, rules=[{"type": "deletion"}, {"type": "update"}])],
            [dict(TAG_RULESET, conditions={"ref_name": {"include": ["refs/tags/release-*"], "exclude": []}})],
            [dict(TAG_RULESET, conditions={"ref_name": {"include": ["~ALL"], "exclude": ["refs/tags/v*"]}})],
        ]
        for rulesets in weak:
            with self.subTest(rulesets=rulesets):
                self.assertTrue(any("tag ruleset" in p for p in problems(REPO, github(rulesets=rulesets))))
        for include in (["~ALL"], ["refs/tags/*"], ["refs/tags/v*"]):
            ruleset = dict(TAG_RULESET, conditions={"ref_name": {"include": include, "exclude": []}})
            with self.subTest(include=include):
                self.assertEqual(problems(REPO, github(rulesets=[ruleset])), [])


if __name__ == "__main__":
    unittest.main()
