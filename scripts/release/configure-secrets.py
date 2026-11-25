#!/usr/bin/env python3
"""Run in your own terminal to provision the GitHub release environment."""
import argparse
import base64
import fnmatch
import getpass
import json
import subprocess
from pathlib import Path

# A tag that must count as a release tag for the ruleset check.
SAMPLE_TAG = "refs/tags/v1.0.0"


def gh_api(path):
    """GET a GitHub API path through gh; None if it fails (for example, 404)."""
    result = subprocess.run(["gh", "api", path], capture_output=True, text=True)
    return json.loads(result.stdout) if result.returncode == 0 else None


def protects_release_tags(ruleset):
    """True if an active tag ruleset stops non-maintainers creating, moving or deleting v* tags."""
    names = ruleset.get("conditions", {}).get("ref_name", {})
    covered = lambda patterns: any(p == "~ALL" or fnmatch.fnmatchcase(SAMPLE_TAG, p) for p in patterns)
    rules = {rule.get("type") for rule in ruleset.get("rules", [])}
    return (ruleset.get("target") == "tag" and ruleset.get("enforcement") == "active"
            and covered(names.get("include", [])) and not covered(names.get("exclude", []))
            and {"creation", "update", "deletion"} <= rules)


def release_protection_problems(repository, api=gh_api):
    """What is missing that stops a pushed tag alone from reaching the signing secrets.

    The workflow's own "tag must be on main" check runs from the tagged commit,
    so whoever pushes a tag also chooses that check. Only GitHub-side settings
    hold: who may create v* tags, and a reviewer before each signing job.
    """
    env = api(f"repos/{repository}/environments/release")
    if env is None:
        return ["create the release environment with tag-only deployment access first; see RELEASES.md"]
    problems = []
    policy = env.get("deployment_branch_policy") or {}
    if not policy.get("custom_branch_policies"):
        problems.append("release environment must restrict deployment refs; see RELEASES.md")
    else:
        allowed = api(f"repos/{repository}/environments/release/deployment-branch-policies?per_page=100") or {}
        if allowed.get("total_count") != 1 or [(rule.get("type"), rule.get("name")) for rule in allowed.get("branch_policies", [])] != [("tag", "v*")]:
            problems.append("release environment must allow only the v* tag pattern, with no branches")
    reviews = [rule for rule in env.get("protection_rules", []) if rule.get("type") == "required_reviewers"]
    if not any(rule.get("reviewers") for rule in reviews):
        problems.append("release environment must require a reviewer before signing jobs run; see RELEASES.md")
    elif not all(rule.get("prevent_self_review") for rule in reviews):
        problems.append("release environment must prevent self-review, so whoever pushed a tag cannot approve its run alone")
    summaries = api(f"repos/{repository}/rulesets?includes_parents=true&per_page=100") or []
    tag_rulesets = (api(f"repos/{repository}/rulesets/{summary['id']}?includes_parents=true") or {} for summary in summaries if summary.get("target") == "tag")
    if not any(protects_release_tags(ruleset) for ruleset in tag_rulesets):
        problems.append("add an active tag ruleset for refs/tags/v* that restricts creations, updates and deletions to release maintainers; see RELEASES.md")
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("certificate", type=Path, nargs="?", help="encrypted .p12 containing only the Skiff Developer ID identity")
    parser.add_argument("--repository", default="shebashio/skiff")
    parser.add_argument("--check", action="store_true", help="only check the release environment and tag ruleset; upload nothing")
    args = parser.parse_args()
    if not args.check and args.certificate is None:
        parser.error("the certificate is required unless --check is given")
    # Before any prompt: no secret is uploaded to an unprotected environment.
    problems = release_protection_problems(args.repository)
    if problems:
        parser.exit(2, "".join(f"{parser.prog}: {problem}\n" for problem in problems))
    if args.check:
        print("Release environment and tag ruleset protect the signing secrets.")
        return
    certificate = args.certificate.read_bytes()
    if not certificate or len(base64.b64encode(certificate)) > 48000:
        parser.error("certificate must be a nonempty .p12 smaller than GitHub's secret limit")
    password = getpass.getpass("P12 export password: ")
    if not password:
        parser.error("use a password-protected P12")
    # Check the certificate without printing its contents or exporting its key.
    command = ["openssl", "pkcs12", "-in", str(args.certificate), "-clcerts", "-nokeys", "-passin", "stdin"]
    result = subprocess.run(command, input=password + "\n", capture_output=True, text=True)
    # Keychain Access can use RC2 for the certificate bag. OpenSSL 3+ keeps
    # that reader in its legacy provider; no private key is decoded here.
    if result.returncode and "unsupported" in result.stderr.lower():
        result = subprocess.run(command[:2] + ["-legacy"] + command[2:], input=password + "\n", capture_output=True, text=True)
    if result.returncode:
        parser.error("could not decrypt the P12 certificate with that password")
    subject = subprocess.run(["openssl", "x509", "-noout", "-subject"], input=result.stdout, capture_output=True, text=True, check=True).stdout
    if "Developer ID Application:" not in subject:
        parser.error("P12 must contain a Developer ID Application certificate")
    apple_id = input("Apple account email: ").strip()
    team_id = input("Apple developer Team ID: ").strip()
    if not apple_id or not team_id or team_id not in subject:
        parser.error("account is required and Team ID must match the certificate")
    notary_password = getpass.getpass("Apple app-specific password: ")
    if not notary_password:
        parser.error("app-specific password is required")
    secrets = {"APPLE_CERTIFICATE_P12_BASE64": base64.b64encode(certificate).decode(),
               "APPLE_CERTIFICATE_PASSWORD": password, "APPLE_ID": apple_id,
               "APPLE_TEAM_ID": team_id, "APPLE_APP_SPECIFIC_PASSWORD": notary_password}
    for name, value in secrets.items():
        subprocess.run(["gh", "secret", "set", name, "--repo", args.repository, "--env", "release"], input=value, text=True, check=True)
    print("Release signing secrets saved. Passwords were passed to gh through stdin, not command arguments.")


if __name__ == "__main__":
    main()
