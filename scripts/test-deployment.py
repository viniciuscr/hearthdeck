#!/usr/bin/env python3
"""Deployment checks with an isolated home and no real service manager."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
UNITS = (
    "hearthdeck.target",
    "hearthdeck-bridge.socket",
    "hearthdeck-bridge.service",
    "hearthdeck-daemon.service",
    "hearthdeck-input.service",
    "hearthdeck-log.service",
    "hearthdeck-backup.service",
    "hearthdeck-backup.timer",
    "romm.service",
    "romm.path",
)


class DeploymentTests(unittest.TestCase):
    def test_install_only_reloads_and_preserves_existing_enablement(self):
        with tempfile.TemporaryDirectory(prefix=".deployment-test-", dir=ROOT) as temp:
            checkout = Path(temp) / "checkout"
            checkout.mkdir()
            shutil.copy(ROOT / "justfile", checkout)
            for directory in ("deploy/systemd", "scripts"):
                shutil.copytree(ROOT / directory, checkout / directory)
            (checkout / "packaging/arch").mkdir(parents=True)
            shutil.copy(
                ROOT / "packaging/arch/hearthdeck-romm",
                checkout / "packaging/arch/hearthdeck-romm",
            )
            binaries = checkout / "services/target/release"
            binaries.mkdir(parents=True)
            for binary in ("bridge", "daemon", "input", "ai"):
                (binaries / f"hearthdeck-{binary}").write_text("#!/bin/sh\nexit 0\n")
            home = Path(temp) / "home"
            units = home / ".config/systemd/user"
            enabled = units / "default.target.wants"
            enabled.mkdir(parents=True)
            link = enabled / "hearthdeck.target"
            link.symlink_to("../hearthdeck.target")
            custom = units / "hearthdeck-daemon.service.d/10-custom.conf"
            custom.parent.mkdir()
            custom.write_text("[Service]\nEnvironment=MY_SETTING=1\n")
            fake_bin = Path(temp) / "bin"
            fake_bin.mkdir()
            fake = fake_bin / "systemctl"
            fake.write_text(
                '#!/bin/sh\nprintf "%s\\n" "$*" >> "$HOME/systemctl.calls"\n'
                '[ "$*" = "--user daemon-reload" ]\n'
            )
            fake.chmod(0o755)
            env = dict(os.environ, HOME=str(home), PATH=f"{fake_bin}:{os.environ['PATH']}")
            for _ in range(2):
                result = subprocess.run(
                    ["just", "install-services"], cwd=checkout, env=env,
                    text=True, capture_output=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(link.is_symlink())
            link.unlink()
            result = subprocess.run(
                ["just", "install-services"], cwd=checkout, env=env,
                text=True, capture_output=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(link.exists())
            self.assertEqual(
                (home / "systemctl.calls").read_text().splitlines(),
                ["--user daemon-reload"] * 3,
            )
            self.assertEqual(custom.read_text(), "[Service]\nEnvironment=MY_SETTING=1\n")
            for unit in UNITS:
                self.assertEqual(
                    (units / unit).read_bytes(),
                    (ROOT / "deploy/systemd" / unit).read_bytes(),
                )
            for name in ("bridge", "daemon", "input"):
                override = units / f"hearthdeck-{name}.service.d/90-hearthdeck-checkout.conf"
                self.assertEqual(
                    override.read_text(),
                    f"[Service]\nExecStart=\nExecStart=%h/.local/bin/hearthdeck-{name}\n",
                )
            romm = units / "romm.service.d/90-hearthdeck-checkout.conf"
            self.assertEqual(
                romm.read_text(),
                "[Service]\nExecStart=\nExecStart=%h/.local/bin/hearthdeck-romm up\n"
                "ExecStop=\nExecStop=%h/.local/bin/hearthdeck-romm down\n",
            )
            backup = units / "hearthdeck-backup.service.d/90-hearthdeck-checkout.conf"
            self.assertEqual(
                backup.read_text(),
                "[Service]\nExecStart=\n"
                f"ExecStart={checkout}/deploy/backup/hearthdeck-backup\n"
                f"Environment=HEARTHDECK_BACKUP_DEFAULTS={checkout}/deploy/backup\n",
            )
            migration = subprocess.run(
                [
                    "bash", "-c",
                    'source "$1"; invoking_user() { echo checkout-user; }; '
                    'getent() { printf "checkout-user:x:1000:1000::%s:/bin/sh\\n" "$HOME"; }; '
                    "clear_checkout_units",
                    "test", str(ROOT / "packaging/arch/hearthdeck.install"),
                ],
                env=env, text=True, capture_output=True,
            )
            self.assertEqual(migration.returncode, 0, migration.stderr)
            self.assertEqual(custom.read_text(), "[Service]\nEnvironment=MY_SETTING=1\n")
            for unit in UNITS:
                self.assertFalse((units / unit).exists())
                self.assertTrue((units / f"{unit}.pre-package.bak").exists())
            for unit in ("hearthdeck-bridge.service", "hearthdeck-daemon.service",
                         "hearthdeck-input.service", "hearthdeck-backup.service",
                         "romm.service"):
                override = units / f"{unit}.d/90-hearthdeck-checkout.conf"
                self.assertFalse(override.exists())
                self.assertTrue(override.with_suffix(".conf.pre-package.bak").exists())
            (binaries / "hearthdeck-ai").unlink()
            result = subprocess.run(
                ["just", "install-services"], cwd=checkout, env=env,
                text=True, capture_output=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("run just build-services first", result.stderr)
            self.assertFalse((units / "hearthdeck.target").exists())
            self.assertEqual(
                (home / "systemctl.calls").read_text().splitlines(),
                ["--user daemon-reload"] * 3,
            )

    def test_package_units_resolve_to_the_canonical_sources(self):
        for unit in (ROOT / "packaging/arch").glob("*"):
            if unit.suffix in (".service", ".socket", ".target"):
                self.assertEqual(unit.resolve(), ROOT / "deploy/systemd" / unit.name)
        romm = (ROOT / "deploy/systemd/romm.service").read_text()
        self.assertIn("ExecStart=/usr/lib/hearthdeck/hearthdeck-romm up", romm)
        self.assertIn("ExecStop=/usr/lib/hearthdeck/hearthdeck-romm down", romm)
        self.assertIn(
            "PathExists=/mnt/external/romM/podman-compose.yaml",
            (ROOT / "deploy/systemd/romm.path").read_text(),
        )


if __name__ == "__main__":
    unittest.main()
