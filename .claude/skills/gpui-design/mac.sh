#!/usr/bin/env bash
# Drives the real dotori windows on macOS without touching the owner's screen, mouse or keyboard.
# The window opens off every screen and never takes focus, and the owner's side is spoken over the
# daemon's window link, as a window speaks. dotori runs with <home> as its HOME, so a command the
# fake model asks to run, such as the default mv of ~/Downloads, never reaches the owner's files.
#   mac.sh start <home> [command purpose]   build, then start the fake model and dotori; the fake model asks to run command
#   mac.sh say <home> <message>             say something as the owner
#   mac.sh allow <home> | decline <home>    answer the call that waits on the owner
#   mac.sh shot <home> <out.png> [look] [language]   open the chat window unseen and save it
#   mac.sh stop <home>                      stop what start started
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
target="${CARGO_TARGET_DIR:-$repo/target}"
dotori="$target/debug/dotori"
model="$target/debug/examples/demo_model"
owner="$target/debug/examples/demo_owner"
offline="http://127.0.0.1:9"

usage() { sed -n '2,10p' "$0" >&2; exit 2; }
[ $# -ge 2 ] || usage
command="$1"
home="$(mkdir -p "$2" && cd "$2" && pwd)"
pids="$home/demo.pids"

background() {
  local log="$1"; shift
  nohup "$@" >"$log" 2>&1 </dev/null &
  echo $! >>"$pids"
}

window_id() {
  swift - "$1" <<'SWIFT'
import CoreGraphics
let pid = Int32(CommandLine.arguments[1])!
let windows = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]] ?? []
for window in windows where window[kCGWindowOwnerPID as String] as? Int32 == pid
  && window[kCGWindowLayer as String] as? Int == 0 {
  print(window[kCGWindowNumber as String]!)
  break
}
SWIFT
}

case "$command" in
start)
  (cd "$repo" && cargo build --quiet -p dotori --bin dotori --example demo_model --example demo_owner)
  rm -f "$home/model.port"
  background "$home/model.log" "$model" "$home/model.port" "${@:3}"
  for _ in $(seq 50); do [ -s "$home/model.port" ] && break; sleep 0.1; done
  [ -s "$home/model.port" ] || { echo "the fake model did not start; see $home/model.log" >&2; exit 1; }
  port="$(cat "$home/model.port")"
  cat >"$home/settings.toml" <<TOML
[model]
provider = "openai-compatible"
base_url = "http://127.0.0.1:$port/v1"
model = "fake"
api_key_env = "DOTORI_DEMO_KEY"

[browser]
path = "no-browser-in-demo"
TOML
  rm -f "$home/door.json"
  background "$home/dotori.log" \
    env HOME="$home" DOTORI_HOME="$home" DOTORI_DEMO_KEY=fake DOTORI_TEST_WITHOUT_TRAY=1 \
    HTTPS_PROXY="$offline" HTTP_PROXY="$offline" ALL_PROXY="$offline" NO_PROXY=127.0.0.1,localhost \
    "$dotori"
  for _ in $(seq 100); do [ -s "$home/door.json" ] && break; sleep 0.1; done
  [ -s "$home/door.json" ] || { echo "dotori did not start; see $home/dotori.log" >&2; exit 1; }
  echo "ready: $home"
  ;;
say)
  [ $# -ge 3 ] || usage
  "$owner" "$home" say "$3"
  ;;
allow|decline)
  "$owner" "$home" "$command"
  ;;
shot)
  [ $# -ge 3 ] || usage
  out="$3"; look="${4:-light}"; language="${5:-ko}"
  [ -f "$home/window.pid" ] && kill "$(cat "$home/window.pid")" 2>/dev/null || true
  (HOME="$home" DOTORI_HOME="$home" DOTORI_THEME="$look" DOTORI_LANGUAGE="$language" DOTORI_WINDOW_UNSEEN=1 \
    nohup "$dotori" window >"$home/window.log" 2>&1 </dev/null & echo $! >"$home/window.pid")
  window=""
  for _ in $(seq 100); do
    window="$(window_id "$(cat "$home/window.pid")")"
    [ -n "$window" ] && break
    sleep 0.1
  done
  [ -n "$window" ] || { echo "no window; see $home/window.log" >&2; exit 1; }
  sleep 2
  screencapture -x -o -l "$window" "$out"
  kill "$(cat "$home/window.pid")" 2>/dev/null || true
  rm -f "$home/window.pid"
  echo "saved: $out"
  ;;
stop)
  [ -f "$home/window.pid" ] && kill "$(cat "$home/window.pid")" 2>/dev/null || true
  if [ -f "$pids" ]; then
    while read -r pid; do kill "$pid" 2>/dev/null || true; done <"$pids"
    rm -f "$pids"
  fi
  ;;
*)
  usage
  ;;
esac
