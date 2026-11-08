#!/bin/sh
# A release's notes: what changed for its users since the previous release, from the commit subjects (AGENTS.md,
# "Versioning"). "New" lists the `feat:` commits and "Fixed" the `fix:` ones; docs, tests, CI and chores are left out.
# Markdown, for the GitHub release; site/build.py turns the latest ones into the Mac update window's notes. Nothing is
# printed when nothing changed for users.
#
#   ci/release-notes.sh [PREVIOUS_TAG]     (since the root commit when no tag is given)
set -eu
range=${1:+$1..}HEAD
# The subject without its type, scope and breaking mark, its first letter capitalised.
subjects() {
  git log --no-merges --format=%s "$range" |
    sed -n "s/^$1\(([^)]*)\)\{0,1\}!\{0,1\}: //p" |
    awk '{ print "- " toupper(substr($0, 1, 1)) substr($0, 2) }'
}
new=$(subjects feat)
fixed=$(subjects fix)
if [ -n "$new" ]; then
  printf '### New\n\n%s\n' "$new"
fi
if [ -n "$fixed" ]; then
  [ -n "$new" ] && printf '\n'
  printf '### Fixed\n\n%s\n' "$fixed"
fi
