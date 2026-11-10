#!/bin/sh
# Builds the Files extension against this system's Nautilus: build.sh <output .so> [locale dir].
# Each distribution's package builds its own, against the Nautilus it installs beside (release.yml).
set -eu
here=$(dirname "$0")
${CC:-cc} -shared -fPIC -fvisibility=hidden -O2 -Wall -Wextra -Werror -std=gnu11 \
  -DLOCALEDIR="\"${2:-/usr/share/locale}\"" \
  "$here/immich-mount-nautilus.c" -o "$1" \
  -Wl,--as-needed -Wl,-z,defs $(pkg-config --cflags --libs libnautilus-extension-4 gtk4 gio-unix-2.0)
