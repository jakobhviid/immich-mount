#!/bin/bash
# In-VM, as root: record which fixture files get opened during one experiment.
#
#   watch.sh start <name>   start the watchers; logs go to /root/ev/<name>.*
#   watch.sh stop <name>    stop them and print a summary
#
# Two watchers, because they answer different questions:
#   inotifywait on the SOURCE dir (<name>.inotify): which files the FUSE daemon
#     (bindfs) had to open/read, i.e. what our daemon would have to download.
#   bpftrace on the kernel's FUSE open/read (<name>.fuse): which PROCESS opened
#     or read the file through the mount (nautilus, a thumbnailer, localsearch …).
#     fatrace/fanotify does NOT see accesses through a non-allow_other FUSE mount.
set -euo pipefail
src=${SRC:-/srv/photos}
cmd=${1:?start|stop}; name=${2:?name}
mkdir -p /root/ev
log=/root/ev/$name

case $cmd in
start)
  systemctl stop ev-inotify ev-fuse 2>/dev/null || true
  systemctl reset-failed ev-inotify ev-fuse 2>/dev/null || true
  rm -f "$log".inotify "$log".fuse
  since=$(date '+%Y-%m-%d %H:%M:%S')
  systemd-run --quiet --unit=ev-inotify --collect \
    inotifywait -m -r -e open,access --timefmt '%T' --format '%T %e %w%f' -o "$log.inotify" "$src"
  systemd-run --quiet --unit=ev-fuse --collect \
    sh -c "BPFTRACE_STRLEN=128 exec bpftrace -B line /opt/exp/fuse-opens.bt > $log.fuse 2>&1"
  # Both report readiness: inotifywait on stderr (the journal), bpftrace in its log.
  for _ in $(seq 120); do
    journalctl -q -u ev-inotify --since "$since" | grep -q 'Watches established' \
      && grep -q '^Attached' "$log.fuse" 2>/dev/null && break
    for u in ev-inotify ev-fuse; do
      systemctl -q is-active $u || { echo "$u died:"; journalctl -u $u | tail -5; cat "$log.fuse"; exit 1; }
    done
    sleep 0.5
  done
  grep -q '^Attached' "$log.fuse" || { echo "watchers not ready after 60 s"; exit 1; }
  echo "watching $src (inotify) and FUSE opens/reads (bpftrace) -> $log.*"
  ;;
stop)
  sleep 1  # let the last events land
  systemctl stop ev-inotify ev-fuse 2>/dev/null || true
  touch "$log.inotify" "$log.fuse"
  echo "== inotify on $src: events by type"
  awk '{print $2}' "$log.inotify" | sort | uniq -c
  echo "== inotify: distinct .jpg files OPENed in the source: $(awk '$2 == "OPEN" && /\.jpg$/ {print $3}' "$log.inotify" | sort -u | grep -c . || true)"
  echo "== FUSE layer, per process<-parent (thread names in brackets): distinct .jpg opened, opens, reads, MB requested"
  awk '/\.jpg/ && ($2 == "O" || $2 == "R") {
         p = ($2 == "O") ? $5 : $6; t = $3; sub(/\([0-9]+\)$/, "", t); thr[p][t] = 1
         if ($2 == "O") { opens[p]++; if (!seen[p SUBSEP $4]++) files[p]++ }
         else { reads[p]++; split($5, a, "@"); bytes[p] += a[1] }
         procs[p] = 1 }
       END { for (p in procs) { ts = ""; for (t in thr[p]) ts = ts (ts ? "," : "") t
               printf "  %-32s files=%-3d opens=%-4d reads=%-5d req=%.1fMB [%s]\n",
               p, files[p], opens[p], reads[p], bytes[p] / 1048576, ts } }' "$log.fuse"
  echo "== FUSE layer: directory opens by process<-parent"
  awk '$2 == "D" {print $5}' "$log.fuse" | sort | uniq -c
  ;;
esac
