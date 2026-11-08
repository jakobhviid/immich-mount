#!/usr/bin/env bash
# apple/dev-app.sh in a Terminal window of the developer's session, where signing reaches the login keychain; its
# output also goes to apple/build/dev-app.log, for a reader over SSH. The window stays open afterwards.
cd "$(dirname "$0")" && mkdir -p build && zsh -lc './dev-app.sh' 2>&1 | tee build/dev-app.log
echo "exit ${PIPESTATUS[0]}" | tee -a build/dev-app.log
