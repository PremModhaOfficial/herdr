#!/bin/sh
# managed by herdr; reinstalling the integration replaces this file.
# HERDR_INTEGRATION_ID=jcode
# HERDR_INTEGRATION_VERSION=1
#
# Bridges jcode lifecycle hooks to herdr. jcode invokes this script for
# session_start/turn_start/turn_end/session_end with JCODE_HOOK_EVENT set;
# the script no-ops outside herdr panes.

event="${JCODE_HOOK_EVENT:-}"
case "$event" in
  session_start|turn_start|turn_end|session_end) ;;
  *) exit 0 ;;
esac

[ "${HERDR_ENV:-}" = "1" ] || exit 0
[ -n "${HERDR_SOCKET_PATH:-}" ] || exit 0
[ -n "${HERDR_PANE_ID:-}" ] || exit 0
command -v python3 >/dev/null 2>&1 || exit 0

export JCODE_HOOK_EVENT JCODE_HOOK_SESSION_ID JCODE_HOOK_SOURCE
export HERDR_SOCKET_PATH HERDR_PANE_ID

python3 - <<'PY' 2>/dev/null || true
import json
import os
import socket
import time

source = "herdr:jcode"
agent = "jcode"

pane_id = os.environ.get("HERDR_PANE_ID")
socket_path = os.environ.get("HERDR_SOCKET_PATH")
if not pane_id or not socket_path:
    raise SystemExit(0)

event = os.environ.get("JCODE_HOOK_EVENT", "")
session_id = os.environ.get("JCODE_HOOK_SESSION_ID") or None


def send(method, **params):
    request = {
        "id": f"{source}:{time.time_ns()}",
        "method": method,
        "params": {
            "pane_id": pane_id,
            "source": source,
            "agent": agent,
            "seq": time.time_ns(),
            **params,
        },
    }
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
            client.settimeout(0.5)
            client.connect(socket_path)
            client.sendall((json.dumps(request) + "\n").encode())
            try:
                client.recv(4096)
            except Exception:
                pass
    except Exception:
        pass


if event == "session_start":
    params = {}
    if session_id:
        params["agent_session_id"] = session_id
    # jcode reports create/attach/resume; herdr accepts startup/resume.
    start_source = os.environ.get("JCODE_HOOK_SOURCE") or None
    if start_source in ("create", "attach"):
        params["session_start_source"] = "startup"
    elif start_source == "resume":
        params["session_start_source"] = "resume"
    if params:
        send("pane.report_agent_session", **params)
    send("pane.report_agent", state="idle")
elif event == "turn_start":
    send("pane.report_agent", state="working")
elif event == "turn_end":
    send("pane.report_agent", state="idle")
elif event == "session_end":
    send("pane.report_agent", state="idle")
    send("pane.release_agent")
PY
exit 0
