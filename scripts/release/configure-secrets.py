#!/usr/bin/env python3
"""Run in your own terminal to provision the GitHub release environment."""
import argparse
import base64
import getpass
import json
import subprocess
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("certificate", type=Path, help="encrypted .p12 containing only the Skiff Developer ID identity")
    parser.add_argument("--repository", default="shebashio/skiff")
    args = parser.parse_args()
    certificate = args.certificate.read_bytes()
    if not certificate or len(base64.b64encode(certificate)) > 48000:
        parser.error("certificate must be a nonempty .p12 smaller than GitHub's secret limit")
    password = getpass.getpass("P12 export password: ")
    if not password:
        parser.error("use a password-protected P12")
    # Check the certificate without printing its contents or exporting its key.
    result = subprocess.run(["openssl", "pkcs12", "-in", str(args.certificate), "-clcerts", "-nokeys", "-passin", "stdin"], input=password + "\n", capture_output=True, text=True)
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
    existing = subprocess.run(["gh", "api", f"repos/{args.repository}/environments/release"], capture_output=True, text=True)
    if existing.returncode:
        parser.error("create the release environment with tag-only deployment access first; see RELEASES.md")
    policy = json.loads(existing.stdout).get("deployment_branch_policy") or {}
    if not policy.get("custom_branch_policies"):
        parser.error("release environment must restrict deployment refs; see RELEASES.md")
    rules = subprocess.run(["gh", "api", f"repos/{args.repository}/environments/release/deployment-branch-policies?per_page=100"], capture_output=True, text=True, check=True)
    allowed = json.loads(rules.stdout)
    if allowed.get("total_count") != 1 or [(rule.get("type"), rule.get("name")) for rule in allowed.get("branch_policies", [])] != [("tag", "v*")]:
        parser.error("release environment must allow only the v* tag pattern, with no branches")
    for name, value in secrets.items():
        subprocess.run(["gh", "secret", "set", name, "--repo", args.repository, "--env", "release"], input=value, text=True, check=True)
    print("Release signing secrets saved. Passwords were passed to gh through stdin, not command arguments.")


if __name__ == "__main__":
    main()
