#!/usr/bin/env bash
# Reproduce the thumbnail experiments behind ARCHITECTURE.md, "Thumbnails", on a VM made by up.sh + session.sh,
# with bindfs standing in for the daemon. Screenshots go to the git-ignored screenshots/.
#
#   experiment.sh fixtures
#       40 JPEGs in /srv/photos/2024/03 (4 over 5 MB, EXIF dates), and a bindfs
#       (fuse.bindfs) mount of /srv/photos at ~tester/Immich, without allow_other.
#   experiment.sh open <name> <dir-in-vm> [keep]
#       Quit Nautilus, clear tester's thumbnail cache (unless "keep"), start the
#       watchers, open <dir> in Nautilus, wait until the thumbnail cache settles,
#       screenshot to screenshots/<name>.png, print who opened which files.
#       Logs stay in the VM under /root/ev/<name>.*
#
#   e.g. E3:  experiment.sh open e3 /home/tester/Immich/2024/03
#        E4:  run-in-session.sh python3 /opt/exp/seed-thumbs.py /home/tester/Immich/2024/03 \
#               /srv/photos/2024/03 normal:128 large:256 x-large:512 xx-large:1024
#             experiment.sh open e4 /home/tester/Immich/2024/03 keep
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"
is() { vm /usr/local/bin/in-session "$@"; }

case ${1:?fixtures|open} in
fixtures)
  vm /opt/exp/make-fixtures.sh /srv/photos
  is sh -c 'mkdir -p ~/Immich && { findmnt ~/Immich >/dev/null || bindfs --no-allow-other /srv/photos ~/Immich; }'
  vm findmnt -n /home/$TESTER/Immich
  ;;
open)
  name=${2:?name} dir=${3:?dir in the VM} keep=${4:-}
  is sh -c 'nautilus -q 2>/dev/null; true'
  wait_for "$P" "$VM" "nautilus to exit" sh -c "! pgrep -u $TESTER -x nautilus"
  [ "$keep" = keep ] || vm rm -rf "/home/$TESTER/.cache/thumbnails"
  vm /opt/exp/watch.sh start "$name"
  step "opening $dir"
  is -d nautilus --new-window "$dir"
  wait_for "$P" "$VM" "the Nautilus window" sh -c "/opt/exp/windows.sh | grep -q '^org.gnome.Nautilus|'"
  vm /opt/exp/settle.sh 8 240
  mkdir -p "$here/screenshots"
  "$here/shot.sh" "$here/screenshots/$name.png"
  vm /opt/exp/watch.sh stop "$name"
  ;;
*) die "usage: $0 fixtures | open <name> <dir> [keep]" ;;
esac
