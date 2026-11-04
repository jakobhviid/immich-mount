#!/usr/bin/env bash
# How long opening a folder keeps Files busy on Bazzite GNOME, where rom-properties reads every file of a folder shown
# from Files' main thread. Closes Files, drops the page cache (reads the kernel answers from memory never reach the
# daemon), starts Files on Home, then opens the folder and samples Files' main thread (lab/gnome-vm/exp/files-freeze.sh).
#
#   lab/bazzite-vm/freeze.sh "Photos/1997/01 January" [seconds]
#
# Measure a folder the daemon has not seen since it started, or the edges and seeds of an earlier visit answer it:
# lab/immich-server/bulk.sh makes as many fresh months as needed.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"
pick_vm
[ $# -ge 1 ] || die "usage: $0 <folder under ~/Immich> [seconds]"
vm bash -c 'pkill -x nautilus; for attempt in $(seq 50); do pgrep -x nautilus >/dev/null || break; sleep 0.1; done
  sync; echo 3 > /proc/sys/vm/drop_caches'
"$here/run-in-session.sh" -d nautilus -w /home/tester >/dev/null
"$here/run-in-session.sh" bash -c 'for attempt in $(seq 100); do
  gdbus call --session --dest org.gnome.Nautilus --object-path /org/gnome/Nautilus --method org.gtk.Actions.List \
    >/dev/null 2>&1 && exit 0; sleep 0.1; done; exit 1' || die "Files did not start"
"$here/run-in-session.sh" /opt/exp/files-freeze.sh "$1" "${2:-30}"
