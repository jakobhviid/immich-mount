#!/bin/bash
# In-VM, as root: print one line per window in the headless shell: "<wm_class>|<title>".
# Uses org.gnome.Shell.Eval (needs --unsafe-mode). Exits 1 if the shell is not up.
#   windows.sh | grep -q '^org.gnome.Nautilus|'
set -euo pipefail
out=$(/usr/local/bin/in-session gdbus call --session --dest org.gnome.Shell \
  --object-path /org/gnome/Shell --method org.gnome.Shell.Eval \
  'global.get_window_actors().map(a => a.meta_window.get_wm_class() + "|" + a.meta_window.get_title()).join(" ;; ")')
# Eval returns (true, '<JSON-encoded result>'), e.g. (true, '"org.gnome.Nautilus|Home ;; …"').
# A literal "\n" in the JS would reach the shell as a raw line break, hence " ;; ".
case "$out" in "(true, "*) ;; *) echo "$out" >&2; exit 1 ;; esac
out=${out#(true, \'}; out=${out%\')}
[ -z "$out" ] || printf '%s\n' "$out" | sed 's/^"//; s/"$//; s/ ;; /\n/g'
