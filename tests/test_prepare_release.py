import importlib.util
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).parent.parent / "scripts" / "prepare-release.py"
SPEC = importlib.util.spec_from_file_location("prepare_release", SCRIPT)
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class PrepareReleaseTests(unittest.TestCase):
    def test_classifies_conventional_commits(self):
        self.assertEqual(release.bump_for_message("fix: handle failure"), "patch")
        self.assertEqual(release.bump_for_message("feat(picker): add search"), "minor")
        self.assertEqual(release.bump_for_message("feat!: replace API"), "major")
        self.assertEqual(
            release.bump_for_message("fix: change behavior\n\nBREAKING CHANGE: new format"),
            "major",
        )
        self.assertIsNone(release.bump_for_message("docs: update README"))
        self.assertIsNone(release.bump_for_message("not conventional"))

    def test_uses_highest_required_bump(self):
        self.assertEqual(
            release.required_bump(["fix: one", "feat: two", "docs: three"]),
            "minor",
        )
        self.assertIsNone(release.required_bump(["docs: one", "ci: two"]))

    def test_bumps_semantic_versions(self):
        self.assertEqual(release.bump_version("1.2.3", "patch"), "1.2.4")
        self.assertEqual(release.bump_version("1.2.3", "minor"), "1.3.0")
        self.assertEqual(release.bump_version("1.2.3", "major"), "2.0.0")

    def test_updates_all_version_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text(
                '[package]\nname = "herdr-worktree-picker"\nversion = "0.3.2"\n'
            )
            (root / "Cargo.lock").write_text(
                '[[package]]\nname = "herdr-worktree-picker"\nversion = "0.3.2"\n'
            )
            (root / "herdr-plugin.toml").write_text('version = "0.3.2"\n')
            (root / "README.md").write_text("install --ref v0.3.2\n")

            release.update_versions(root, "0.3.2", "0.4.0")

            for path in ["Cargo.toml", "Cargo.lock", "herdr-plugin.toml", "README.md"]:
                self.assertIn("0.4.0", (root / path).read_text())
                self.assertNotIn("0.3.2", (root / path).read_text())


if __name__ == "__main__":
    unittest.main()
