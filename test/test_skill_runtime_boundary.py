import json
import os
import pathlib
import stat
import subprocess
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]


class SkillRuntimeBoundary(unittest.TestCase):
    def test_production_startup_has_no_skill_manager_dependency(self):
        session = (ROOT / "desktop/src-tauri/src/runtime/sandbox_session.rs").read_text()
        for forbidden in (
            "skill_manager",
            "commands::skills",
            ".claude/skills",
            "scan_and_reconcile",
            "CSSWITCH_RECONCILED_DATA_DIR",
            "STORE_CONFLICT",
            "LIMIT_EXCEEDED",
        ):
            self.assertNotIn(forbidden, session)

        lib = (ROOT / "desktop/src-tauri/src/lib.rs").read_text()
        command_block = lib.split("tauri::generate_handler![", 1)[1].split("])", 1)[0]
        self.assertNotIn("commands::skills", command_block)
        self.assertNotIn("mod skill_manager;", lib)

        commands = (ROOT / "desktop/src-tauri/src/commands/mod.rs").read_text()
        self.assertNotIn("mod skills;", commands)

        catalog = json.loads((ROOT / "catalog/capabilities.v1.json").read_text())
        self.assertEqual(catalog["skills"], [])

    def test_gateway_starts_only_after_config_and_science_state_prechecks(self):
        session = (ROOT / "desktop/src-tauri/src/runtime/sandbox_session.rs").read_text()
        state_check = session.index("match sandbox_science_state(sport)")
        self.assertLess(session.index("config::load_from(&dir)"), state_check)
        self.assertNotIn("ensure_proxy(", session[:state_check])

        launch_check = session.index("if !launch.is_file()")
        normal_proxy = session.index("let (pport, secret, proxy_action) = ensure_proxy", launch_check)
        self.assertGreater(normal_proxy, launch_check)

    def test_launcher_ignores_large_external_tree_and_broken_legacy_store(self):
        with tempfile.TemporaryDirectory(
            prefix="csswitch-skill-boundary-", dir="/private/tmp"
        ) as raw_tmp:
            tmp = pathlib.Path(raw_tmp)
            outer_home = tmp / "outer-home"
            external = outer_home / ".claude" / "skills"
            legacy_store = outer_home / ".csswitch" / "skills"
            external.mkdir(parents=True)
            legacy_store.mkdir(parents=True)
            for index in range(300):
                skill = external / f"skill-{index:03d}"
                skill.mkdir()
                (skill / "SKILL.md").write_text(
                    f"---\nname: skill-{index:03d}\ndescription: boundary probe\n---\n"
                )
            (legacy_store / "inventory.v1.json").write_text("{broken")

            bin_dir = tmp / "bin"
            bin_dir.mkdir()
            security = bin_dir / "security"
            security.write_text("#!/bin/sh\nexit 0\n")
            security.chmod(0o700)

            marker = tmp / "science-invocation.txt"
            science = tmp / "fake-claude-science"
            science.write_text(
                "#!/bin/sh\n"
                "printf 'HOME=%s\\n' \"$HOME\" > \"$CSSWITCH_TEST_MARKER\"\n"
                "printf 'ARGS=%s\\n' \"$*\" >> \"$CSSWITCH_TEST_MARKER\"\n"
                "exit 0\n"
            )
            science.chmod(0o700)

            sandbox_home = tmp / "sandbox-home"
            data_dir = sandbox_home / ".claude-science"
            existing_skill = data_dir / "orgs" / "org-v043" / "skills" / "existing-skill"
            existing_skill.mkdir(parents=True)
            existing_skill_bytes = (
                b"---\nname: existing-skill\ndescription: v0.4.3 upgrade probe\n---\n"
            )
            (existing_skill / "SKILL.md").write_bytes(existing_skill_bytes)
            active_org_bytes = b'{"org_uuid":"org-v043"}\n'
            (data_dir / "active-org.json").write_bytes(active_org_bytes)
            env = os.environ.copy()
            env.update(
                {
                    "HOME": str(outer_home),
                    "SANDBOX_HOME": str(sandbox_home),
                    "SCIENCE_BIN": str(science),
                    "CSSWITCH_TEST_MARKER": str(marker),
                    "PATH": f"{bin_dir}:/usr/bin:/bin:/usr/sbin:/sbin",
                }
            )

            external.chmod(0)
            try:
                result = subprocess.run(
                    [
                        str(ROOT / "scripts/launch-virtual-sandbox.sh"),
                        "--port",
                        "19932",
                        "--proxy-url",
                        "http://127.0.0.1:19931/test-secret",
                        "--skip-oauth-forge",
                    ],
                    env=env,
                    capture_output=True,
                    text=True,
                    timeout=15,
                    check=False,
                )
            finally:
                external.chmod(stat.S_IRWXU)

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            invocation = marker.read_text()
            self.assertIn(f"HOME={sandbox_home}", invocation)
            self.assertIn(
                f"--data-dir {sandbox_home / '.claude-science'}", invocation
            )
            self.assertEqual(
                (existing_skill / "SKILL.md").read_bytes(), existing_skill_bytes
            )
            self.assertEqual(
                (data_dir / "active-org.json").read_bytes(), active_org_bytes
            )
            self.assertNotIn("LIMIT_EXCEEDED", result.stdout + result.stderr)
            self.assertNotIn("STORE_CONFLICT", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
