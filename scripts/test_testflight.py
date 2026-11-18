"""Signing preflight checks, using synthetic profiles (no signing credentials)."""
import copy
import datetime
import unittest
import testflight


class ProfileTests(unittest.TestCase):
    def profile(self):
        return {
            "ExpirationDate": datetime.datetime.now() + datetime.timedelta(days=1),
            "TeamIdentifier": [testflight.TEAM],
            "Platform": ["OSX"],
            "Entitlements": {
                "com.apple.application-identifier": f"{testflight.TEAM}.{testflight.BUNDLE}",
                "com.apple.security.virtualization": True,
            },
        }

    def test_mac_distribution_profile_does_not_require_ios_beta_entitlement(self):
        profile = self.profile()
        self.assertIs(testflight.validate_profile(profile), profile)

    def test_rejects_wrong_team_bundle_platform_or_missing_virtualization(self):
        original = self.profile()
        for key, value in (("TeamIdentifier", ["OTHER"]), ("Platform", ["iOS"]),
                           ("ExpirationDate", datetime.datetime(2000, 1, 1))):
            with self.subTest(key=key), self.assertRaises(ValueError):
                testflight.validate_profile({**original, key: value})
        for key, value in (("com.apple.application-identifier", "OTHER.app"),
                           ("com.apple.security.virtualization", False),
                           ("get-task-allow", True), ("com.apple.security.get-task-allow", True)):
            profile = copy.deepcopy(original)
            profile["Entitlements"][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                testflight.validate_profile(profile)

    def test_rejects_development_and_developer_id_distribution(self):
        for key, value in (("ProvisionedDevices", []), ("ProvisionsAllDevices", True)):
            with self.subTest(key=key), self.assertRaises(ValueError):
                testflight.validate_profile({**self.profile(), key: value})


if __name__ == "__main__":
    unittest.main()
