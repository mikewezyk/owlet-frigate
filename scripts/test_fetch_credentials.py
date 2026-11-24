import stat
import tempfile
import unittest
from pathlib import Path

import fetch_credentials as fc

CREDS = {"tutkid": "ABCDEFGHJKLMNPRSTUVW", "password": "p$w", "authKey": "Auth4321"}


class Tests(unittest.TestCase):
    def test_only_cameras_are_listed(self):
        services = [
            {"serviceType": "SERVICE_TYPE_CAMERA", "device": {"dsn": "OC1"}, "displayName": "Crib"},
            {"serviceType": "SERVICE_TYPE_SOCK", "device": {"dsn": "S1"}},
            {"serviceType": "SERVICE_TYPE_CAMERA", "device": None},
        ]
        self.assertEqual(fc.camera_list(services), [("OC1", "Crib")])

    def test_choice(self):
        two = [("A", "a"), ("B", "b")]
        self.assertEqual(fc.choose(two[:1], None), "A")
        self.assertEqual(fc.choose(two, "B"), "B")
        for cameras, wanted in [(two, None), ([], None), (two, "C")]:
            with self.assertRaises(fc.Problem):
                fc.choose(cameras, wanted)

    def test_env_values_are_quoted(self):
        self.assertIn("OWLET_TUTK_PASSWORD='p$w'\n", fc.env_text("192.0.2.1", CREDS, "r"))

    def test_bad_values_do_not_leak(self):
        for secret in ["a'b", "a\nb", "a\\b"]:
            with self.assertRaises(fc.Problem) as caught:
                fc.env_text("192.0.2.1", {**CREDS, "password": secret}, "r")
            self.assertNotIn(secret, str(caught.exception))

    def test_missing_credentials(self):
        with self.assertRaises(fc.Problem):
            fc.env_text("192.0.2.1", {}, "r")

    def test_file_is_private(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / ".env"
            fc.write_private(path, "x")
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            self.assertEqual([p.name for p in Path(folder).iterdir()], [".env"])


if __name__ == "__main__":
    unittest.main()
