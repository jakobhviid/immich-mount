#!/bin/bash
# In-VM, as root: wait until tester's thumbnail cache stops changing (no new or
# rewritten file for <quiet> seconds), then print what is in it. Bounded: gives
# up after <max> seconds and says so.
#
#   settle.sh [quiet=8] [max=180]
set -euo pipefail
quiet=${1:-8}; max=${2:-180}
dir=/home/tester/.cache/thumbnails
sig() { find "$dir" -type f -printf '%p %s %T@\n' 2>/dev/null | sort | md5sum; }
start=$(date +%s); last=$(sig); since=$start
while :; do
  sleep 1
  now=$(date +%s); cur=$(sig)
  if [ "$cur" != "$last" ]; then last=$cur; since=$now; fi
  [ $((now - since)) -ge "$quiet" ] && break
  if [ $((now - start)) -ge "$max" ]; then echo "settle: cache still changing after ${max}s" >&2; break; fi
done
echo "settled after $((now - start))s (quiet ${quiet}s)"
for d in "$dir"/*/; do
  [ -d "$d" ] || continue
  printf '%-10s %3d files\n' "$(basename "$d")" "$(find "$d" -type f | wc -l)"
done
