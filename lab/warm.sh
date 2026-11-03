# Warm lab instances (AGENTS.md, "Testing on Erebus"). Sourced after lib/erebus.sh, with P set to the project.
#
# An instance stays up for a working session and carries user.expires. Once set up it gets a stateless snapshot,
# "fresh", taken while stopped; each later start restores it, so a check never sees an earlier one's leftovers.
# Restoring and booting the GNOME VM takes about 14 s against 140 s to build it (measured on Erebus). A stateful
# snapshot was slower (16 s), needs the VM's memory as disk, and the desktop session did not come back.

WARM_HOURS=${WARM_HOURS:-12}
FRESH_SNAPSHOT=fresh

# The expiry for an instance started now.
expires_at() { date -u -d "+$WARM_HOURS hours" +%FT%TZ; }

# Give an instance another WARM_HOURS.
renew() { inc config set --project "$P" "$1" user.expires="$(expires_at)" </dev/null; }

# Delete the project's expired instances: ind- names whose user.expires has passed, never one with user.keep=true.
# One without an expiry is left alone: someone may be working with it.
sweep_expired() {
  local now name expires
  now=$(date -u +%s)
  for name in $(inc list --project "$P" -f csv -c n </dev/null 2>/dev/null); do
    case "$name" in ind-*) ;; *) continue ;; esac
    [ "$(inc config get --project "$P" "$name" user.keep </dev/null 2>/dev/null)" = true ] && continue
    expires=$(inc config get --project "$P" "$name" user.expires </dev/null 2>/dev/null)
    [ -n "$expires" ] || continue
    if [ "$(date -u -d "$expires" +%s 2>/dev/null || echo "$now")" -lt "$now" ]; then
      inc delete --project "$P" -f "$name" </dev/null && step "deleted $name (expired $expires)"
    fi
  done
}

# Whether the instance exists with its fresh snapshot.
has_fresh() {
  inc snapshot list --project "$P" "$1" -f csv,noheader -c n </dev/null 2>/dev/null | grep -qx "$FRESH_SNAPSHOT"
}

# Put an instance back to its fresh snapshot and start it. The snapshot restores the config too, so the expiry is
# renewed after it.
restore_fresh() {
  inc stop --project "$P" -f "$1" </dev/null 2>/dev/null || true
  inc snapshot restore --project "$P" "$1" "$FRESH_SNAPSHOT" </dev/null
  renew "$1"
  inc start --project "$P" "$1" </dev/null
}

# Take the fresh snapshot of an instance that is set up: stopped, so the disk is consistent; then start it again.
snapshot_fresh() {
  inc stop --project "$P" "$1" </dev/null
  inc snapshot delete --project "$P" "$1" "$FRESH_SNAPSHOT" </dev/null 2>/dev/null || true
  inc snapshot create --project "$P" "$1" "$FRESH_SNAPSHOT" </dev/null
  inc start --project "$P" "$1" </dev/null
}
