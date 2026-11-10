#!/bin/sh
# Builds Dolphin's plugin against this system's KF6 and Qt: build.sh <output .so>. Each distribution's package builds
# its own, as Qt refuses a plugin built against a newer Qt than the one loading it. Warnings are errors.
set -eu
here=$(dirname "$0")
build=$(mktemp -d)
cmake -S "$here" -B "$build" --log-level=WARNING -DCMAKE_BUILD_TYPE=Release -DBUILD_TESTING=OFF \
  -DCMAKE_COMPILE_WARNING_AS_ERROR=ON
cmake --build "$build"
cp "$build/bin/kf6/kfileitemaction/immich-mount-dolphin.so" "$1"
