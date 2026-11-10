#!/usr/bin/env python3
"""Click the Plasma VM's screen at x y through KWin's fake input protocol (org_kde_kwin_fake_input, the one KDE
Connect uses): the headless KWin reads no input devices, so this is how a click reaches what the accessibility bus
cannot press, such as the system tray's icons, or a right click that opens Dolphin's menu on an item. Run in the
session (in-session click.py x y [right]), as tester."""
import subprocess
import sys
import time
from pathlib import Path

# The protocol's Python bindings, made from its XML on first use (python3-pywayland, plasma-wayland-protocols-devel
# and wayland-devel, which up.sh installs).
BINDINGS = Path.home() / ".cache" / "lab-protocols"
if not (BINDINGS / "fake_input").is_dir():
    BINDINGS.mkdir(parents=True, exist_ok=True)
    subprocess.run([sys.executable, "-m", "pywayland.scanner", "-i", "/usr/share/wayland/wayland.xml",
                    "/usr/share/plasma-wayland-protocols/fake-input.xml", "-o", str(BINDINGS)],
                   check=True, capture_output=True)
sys.path.insert(0, str(BINDINGS))
from pywayland.client import Display  # noqa: E402
from fake_input import OrgKdeKwinFakeInput  # noqa: E402

BUTTONS = {"left": 0x110, "right": 0x111}
x, y = float(sys.argv[1]), float(sys.argv[2])
button = BUTTONS[sys.argv[3] if len(sys.argv) > 3 else "left"]
display = Display()
display.connect()
registry = display.get_registry()
found = {}


def announced(registry, name, interface, version):
    if interface == "org_kde_kwin_fake_input":
        found["input"] = registry.bind(name, OrgKdeKwinFakeInput, version)


registry.dispatcher["global"] = announced
display.roundtrip()
fake = found.get("input")
if fake is None:
    sys.exit("click.py: KWin offers no fake input to this program")
fake.authenticate("lab click", "clicks for the lab's checks")
fake.pointer_motion_absolute(x, y)
display.roundtrip()
time.sleep(0.1)
fake.button(button, 1)
display.roundtrip()
time.sleep(0.05)
fake.button(button, 0)
display.roundtrip()
display.disconnect()
