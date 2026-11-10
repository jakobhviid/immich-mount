# Immich Mount, installed with Homebrew: Plasma sources this at login, so Dolphin finds Open in Immich in a folder
# that holds only Immich Mount's plugin (ARCHITECTURE.md, "Links to Immich"). The packages install the plugin where
# Qt looks anyway, and need no such script.
export QT_PLUGIN_PATH="$HOME/.local/lib/immich-mount/plugins${QT_PLUGIN_PATH:+:$QT_PLUGIN_PATH}"
