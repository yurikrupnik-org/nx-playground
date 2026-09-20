# Zellij workspace — imported by the root justfile.
#
# The layout (manifests/zellij/nx-playground.kdl) carries no absolute paths, so
# every pane inherits the cwd of the zellij process: it MUST be started from the
# repo root. These recipes are the supported way to do that (`just` always runs
# from the justfile's directory).
#
# zesh users (zoxide-backed session manager) get the same session with:
#   zesh connect nx-playground -n "$PWD/manifests/zellij/nx-playground.kdl"
# — `-n/--new-session-with-layout` is the same flag this recipe uses, and it is
# honoured on FIRST create only: zellij ignores a layout when attaching to a
# session that already exists.
#
# Flag trap: `zellij -s NAME -l FILE` does NOT create a session — with `-s`,
# `--layout` means "append these tabs to the RUNNING session NAME" and dies with
# "There is no active session!" when it is absent. Creating needs `-n`.

zellij_layout := justfile_directory() / "manifests/zellij/nx-playground.kdl"
zellij_session := "nx-playground"

# Attach to the repo's zellij workspace, creating it from the layout if needed
[group('dev')]
zj:
    #!/usr/bin/env bash
    set -euo pipefail
    if zellij list-sessions --short --no-formatting 2>/dev/null | grep -qx '{{ zellij_session }}'; then
        exec zellij attach '{{ zellij_session }}'
    fi
    exec zellij --session '{{ zellij_session }}' --new-session-with-layout '{{ zellij_layout }}'

# Kill + delete the workspace session (compose and the kind cluster keep running)
[group('dev')]
zj-kill:
    zellij delete-session --force '{{ zellij_session }}'
