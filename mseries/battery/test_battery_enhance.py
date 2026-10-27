#!/usr/bin/python3 -I
"""enhance.sh --only battery stages the logger, command, app, unit and desktop file and enables the user unit."""
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent


@unittest.skipUnless((HERE / "enhance.sh").is_file(), "enhance.sh stays in the OS image repo")
class BatteryEnhanceTests(unittest.TestCase):
    def test_battery_module_stages_everything(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["bash", str(HERE / "enhance.sh"), "--root", directory, "--only", "battery"],
                           check=True, stdout=subprocess.DEVNULL)
            lib = root / "usr/libexec/mseries/battery"
            for name in ("battery_core.py", "mseries-battery-logger.py", "mseries-battery.py", "mseries-battery-app.py"):
                self.assertTrue((lib / name).is_file(), name)
            self.assertTrue((root / "usr/share/applications/org.mseries.Battery.desktop").is_file())
            self.assertTrue((root / "usr/share/icons/hicolor/scalable/apps/org.mseries.Battery.svg").is_file())
            self.assertTrue((root / "usr/local/bin/mseries-battery").is_symlink())
            self.assertTrue((root / "etc/systemd/user/default.target.wants/mseries-battery-logger.service").is_symlink())


if __name__ == "__main__":
    unittest.main()
