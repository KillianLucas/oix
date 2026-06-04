#!/usr/bin/env python3

from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from codex_package.layout import build_package_dir
from codex_package.layout import prepare_package_dir
from codex_package.layout import validate_package_dir
from codex_package.targets import PACKAGE_VARIANTS
from codex_package.targets import PackageInputs
from codex_package.targets import TARGET_SPECS


class PackageLayoutTest(unittest.TestCase):
    def test_open_interpreter_package_contains_entrypoint_and_managed_codex(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            inputs = PackageInputs(
                entrypoint_bin=touch_executable(root / "interpreter"),
                managed_codex_bin=touch_executable(root / "codex"),
                rg_bin=touch_executable(root / "rg"),
                zsh_bin=None,
                bwrap_bin=touch_executable(root / "bwrap"),
                codex_command_runner_bin=None,
                codex_windows_sandbox_setup_bin=None,
            )
            package_dir = root / "package"

            prepare_package_dir(package_dir, force=False)
            build_package_dir(
                package_dir,
                "1.2.3",
                PACKAGE_VARIANTS["open-interpreter"],
                TARGET_SPECS["x86_64-unknown-linux-musl"],
                inputs,
            )
            validate_package_dir(
                package_dir,
                PACKAGE_VARIANTS["open-interpreter"],
                TARGET_SPECS["x86_64-unknown-linux-musl"],
                include_zsh=False,
            )

            self.assertEqual(
                (package_dir / "bin" / "interpreter").read_text(),
                "interpreter",
            )
            self.assertEqual((package_dir / "bin" / "codex").read_text(), "codex")
            metadata = (package_dir / "codex-package.json").read_text(encoding="utf-8")
            self.assertIn('"entrypoint": "bin/interpreter"', metadata)
            self.assertIn('"managedCodex": "bin/codex"', metadata)


def touch_executable(path: Path) -> Path:
    path.write_text(path.name, encoding="utf-8")
    path.chmod(0o755)
    return path.resolve()


if __name__ == "__main__":
    unittest.main()
