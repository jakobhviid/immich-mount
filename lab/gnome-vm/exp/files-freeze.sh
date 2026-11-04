#!/usr/bin/env bash
# files-freeze.sh <folder under ~/Immich> [seconds]: open the folder in Files, which must be running, and report how
# long Files' main thread could not answer. It is sampled back to back with a D-Bus call only the main thread answers
# (GApplication's org.gtk.Actions.List), which takes about 5 ms when Files is idle; a sample over 20 ms counts as busy.
# Prints one line: the busy time, the longest wait, when Files was last busy (from the click), and what the daemon
# fetched of originals meanwhile. Run it as tester in the session (run-in-session.sh); lab/bazzite-vm/freeze.sh
# starts Files fresh around it.
set -u
export PATH=$PATH:/home/linuxbrew/.linuxbrew/bin
folder=$HOME/Immich/$1 window=${2:-30}
uri="file://$(python3 -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1]))' "$folder")"
fetched() { immich-mount status --json 2>/dev/null | jq -r '"\(.fetched.bytes // 0) \(.fetched.requests // 0)"'; }
read -r bytes_before requests_before < <(fetched)
samples=$(mktemp)
start=$(date +%s%N)
gdbus call --session --dest org.freedesktop.FileManager1 --object-path /org/freedesktop/FileManager1 \
  --method org.freedesktop.FileManager1.ShowFolders "['$uri']" "" >/dev/null
echo "0 $(( ($(date +%s%N) - start) / 1000000 ))" >> "$samples"
end=$((start + window * 1000000000))
while [ "$(date +%s%N)" -lt "$end" ]; do
  sent=$(date +%s%N)
  gdbus call --session --timeout 300 --dest org.gnome.Nautilus --object-path /org/gnome/Nautilus \
    --method org.gtk.Actions.List >/dev/null 2>&1
  answered=$(date +%s%N)
  echo "$(( (sent - start) / 1000000 )) $(( (answered - sent) / 1000000 ))" >> "$samples"
  sleep 0.05
done
read -r bytes_after requests_after < <(fetched)
awk -v bytes=$((bytes_after - bytes_before)) -v requests=$((requests_after - requests_before)) -v folder="$1" '
  { if ($2 > 20) { busy += $2; last = $1 + $2 } if ($2 > longest) longest = $2 }
  END { printf "%s: Files busy %d ms, longest wait %d ms, last busy %d ms after the click; fetched %d KB in %d requests\n",
        folder, busy, longest, last, bytes / 1024, requests }' "$samples"
rm -f "$samples"
