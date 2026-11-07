#!/usr/bin/python3
"""Drive Qt apps in the Plasma VM through accessibility (AT-SPI), by the names people see rather than by pixels.

    act.py apps                        the applications on the accessibility bus
    act.py tree <app>                  every showing object of <app>: role and name, indented
    act.py press <app> <name>          press the button, or toggle the switch, named <name>
    act.py set <app> <name> <text>     replace the text of the field named <name>
    act.py set-nth <app> <role> <n> <text>
                                       replace the text of the <n>th showing field of <role>, counted from 1, for
                                       fields Qt leaves unnamed: its password fields lose their name on the bus
    act.py wait <app> <name> [s]       wait until an object named <name> shows (default 30 s)

<app> is the application's name on the bus, such as immich-mount-kde or dolphin. Run as tester inside the session
(`in-session act.py ...`); session.sh turns on Qt's accessibility for every app. Exits 1 when nothing matches.
"""

import sys
import time
import warnings

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi, GLib  # noqa: E402


def applications():
    desktop = Atspi.get_desktop(0)
    for index in range(desktop.get_child_count()):
        app = desktop.get_child_at_index(index)
        if app is not None:
            yield app


def application(name):
    return next((app for app in applications() if app.get_name() == name), None)


def walk(node, depth=0):
    yield node, depth
    try:
        count = node.get_child_count()
    except GLib.Error:
        return
    for index in range(count):
        try:
            child = node.get_child_at_index(index)
        except GLib.Error:
            continue
        if child is not None:
            yield from walk(child, depth + 1)


def showing(node):
    try:
        return node.get_state_set().contains(Atspi.StateType.SHOWING)
    except GLib.Error:
        return False


def matches(app_name, name):
    app = application(app_name)
    if app is None:
        return
    for node, _ in walk(app):
        try:
            if node.get_name() == name and showing(node):
                yield node
        except GLib.Error:
            continue


def find(app_name, name):
    return next(matches(app_name, name), None)


def editable_in(node):
    """The first editable text field that is `node` or inside it."""
    return next((inner for inner, _ in walk(node) if "EditableText" in inner.get_interfaces()), None)


def found(app_name, name):
    node = find(app_name, name)
    if node is None:
        sys.exit(f"act.py: nothing named {name!r} shows in {app_name}")
    return node


PRESSES = ("Press", "Toggle", "press", "toggle", "click")


def action_names(node):
    try:
        action = node.get_action_iface()
        if action is None:
            return []
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            return [Atspi.Action.get_action_name(action, index) for index in range(action.get_n_actions())]
    except GLib.Error:
        return []


def has_actions(node):
    """Whether it can be pressed: GTK's labels have actions too (copy, select all), so a page's title named like
    its button ("Sign In") is passed over."""
    return any(name in PRESSES for name in action_names(node))


def press(node):
    action = node.get_action_iface()
    if action is None:
        sys.exit(f"act.py: {node.get_name()!r} ({node.get_role_name()}) has no actions")
    # The action interface is the accessible itself in Python, so its own get_name() would answer; and this
    # at-spi's replacement for the deprecated get_action_name is not in its bindings.
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", DeprecationWarning)
        names = [Atspi.Action.get_action_name(action, index) for index in range(action.get_n_actions())]
    for wanted in PRESSES:
        if wanted in names:
            action.do_action(names.index(wanted))
            return
    sys.exit(f"act.py: {node.get_name()!r} offers {names}, none of them a press")


def main(arguments):
    command = arguments[0] if arguments else "help"
    if command == "apps":
        for app in applications():
            print(app.get_name())
    elif command == "tree":
        app = application(arguments[1]) or sys.exit(f"act.py: no application {arguments[1]!r}")
        for node, depth in walk(app):
            if showing(node) or depth == 0:
                print(f"{'  ' * depth}{node.get_role_name()}: {node.get_name()!r}")
    elif command == "press":
        # A page's heading can share its button's name ("Sign In"): take the first match that has actions.
        pressable = next((node for node in matches(arguments[1], arguments[2]) if has_actions(node)), None)
        press(pressable or found(arguments[1], arguments[2]))
    elif command == "set":
        # A label often carries the field's name too, and a labelled container (KWallet's password row) holds the
        # field: take the first match that is, or holds, an editable field.
        node = next(filter(None, map(editable_in, matches(arguments[1], arguments[2]))), None)
        if node is None or not node.get_editable_text_iface().set_text_contents(arguments[3]):
            sys.exit(f"act.py: could not set the text of {arguments[2]!r}")
    elif command == "set-nth":
        app = application(arguments[1]) or sys.exit(f"act.py: no application {arguments[1]!r}")
        fields = [node for node, _ in walk(app) if node.get_role_name() == arguments[2] and showing(node)]
        position = int(arguments[3])
        if not 0 < position <= len(fields) or not fields[position - 1].get_editable_text_iface().set_text_contents(arguments[4]):
            sys.exit(f"act.py: could not set the text of {arguments[2]!r} number {position}")
    elif command == "wait":
        deadline = time.monotonic() + (float(arguments[3]) if len(arguments) > 3 else 30.0)
        while find(arguments[1], arguments[2]) is None:
            if time.monotonic() > deadline:
                sys.exit(f"act.py: {arguments[2]!r} did not show in {arguments[1]}")
            time.sleep(0.5)
    else:
        print(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
