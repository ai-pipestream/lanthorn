#!/bin/sh
# Container entrypoint: dispatch between direct-TUI mode and web-serve mode.
#
#   <no args> / <lanthorn args>   exec lanthorn directly (needs `docker run -it`)
#   serve [lanthorn args...]      exec ttyd wrapping lanthorn — each browser
#                                 connection gets its own lanthorn process
#
# serve-mode knobs (environment):
#   LANTHORN_WEB_PORT         port ttyd listens on (default 7681)
#   LANTHORN_WEB_CREDENTIAL   basic-auth as user:pass (default: no auth —
#                             do not expose an unauthenticated port publicly)
set -eu

if [ "${1:-}" = "serve" ]; then
    shift
    # No story args after `serve` means the picker on the library mount.
    [ "$#" -gt 0 ] || set -- /stories

    set -- lanthorn "$@"
    if [ -n "${LANTHORN_WEB_CREDENTIAL:-}" ]; then
        set -- --credential "$LANTHORN_WEB_CREDENTIAL" "$@"
    fi
    # --writable: ttyd >= 1.7 is read-only by default, which would make the
    # game unplayable. disableLeaveAlert spares players a confirm-on-close
    # dialog; titleFixed names the browser tab.
    exec ttyd --writable \
        --port "${LANTHORN_WEB_PORT:-7681}" \
        -t titleFixed=lanthorn \
        -t disableLeaveAlert=true \
        "$@"
fi

exec lanthorn "$@"
