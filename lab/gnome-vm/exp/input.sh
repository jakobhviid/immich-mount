#!/bin/bash
# In-VM, as root: synthesize input in the headless shell through Clutter virtual
# devices created by org.gnome.Shell.Eval (needs --unsafe-mode).
#
#   input.sh click <x> <y>       left click at screen coordinates
#   input.sh type <text>         type ASCII text (letters, digits, space, . _ - @ : /)
#   input.sh key <keysym-name>   one key, e.g. Return, Escape, BackSpace
#
# What made it work on GNOME 50 (each found the hard way):
#  - the Eval scope has no Clutter global: use imports.gi
#  - timestamps: Clutter.CURRENT_TIME; made-up monotonic times dropped the clicks
#  - one Eval per event, with the devices kept on the shell's global object
set -euo pipefail
ev() {
  local out
  out=$(/usr/local/bin/in-session gdbus call --session --dest org.gnome.Shell \
    --object-path /org/gnome/Shell --method org.gnome.Shell.Eval \
    "const {Clutter} = imports.gi; const seat = Clutter.get_default_backend().get_default_seat();
     global._labPointer ??= seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
     global._labKeyboard ??= seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
     $1; true")
  case "$out" in "(true, "*) ;; *) echo "$out" >&2; exit 1 ;; esac
}
key() {
  ev "global._labKeyboard.notify_keyval(Clutter.CURRENT_TIME, Clutter.KEY_$1, Clutter.KeyState.PRESSED)"
  ev "global._labKeyboard.notify_keyval(Clutter.CURRENT_TIME, Clutter.KEY_$1, Clutter.KeyState.RELEASED)"
}
case ${1:?click|type|key} in
click)
  ev "global._labPointer.notify_absolute_motion(Clutter.CURRENT_TIME, $2, $3)"
  ev "global._labPointer.notify_button(Clutter.CURRENT_TIME, Clutter.BUTTON_PRIMARY, Clutter.ButtonState.PRESSED)"
  ev "global._labPointer.notify_button(Clutter.CURRENT_TIME, Clutter.BUTTON_PRIMARY, Clutter.ButtonState.RELEASED)"
  ;;
type)
  printf '%s' "$2" | sed 's/./&\n/g' | while IFS= read -r c; do
    case "$c" in
      ' ') key space ;; .) key period ;; _) key underscore ;; -) key minus ;;
      @) key at ;; :) key colon ;; /) key slash ;;
      [A-Za-z0-9]) key "$c" ;;
      *) echo "input.sh: cannot type '$c'" >&2; exit 1 ;;
    esac
  done
  ;;
key) key "$2" ;;
esac
