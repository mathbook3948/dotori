#!/usr/bin/env bash
# Drives the real dotori windows on a virtual screen with the end-to-end test's fake model.
#   demo.sh start <home>                    build, then start Xvfb, a window manager, the fake model and the daemon
#   demo.sh talk <home> [message ...]       send messages through `dotori chat` (the demo conversation by default)
#   demo.sh shot <home> <out.png> [look] [frames]   open the chat window and save it; frames > 1 saves out-1.png, out-2.png, ...
#   demo.sh settings <home> <out.png> [look]       open the settings window and save it
#   [look] is light, dark, a theme such as forest, or both as forest-dark
#   demo.sh stop <home>                     stop what start started
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
display="${DEMO_DISPLAY:-97}"
target="${CARGO_TARGET_DIR:-$repo/target}"
dotori="$target/debug/dotori"
model="$target/debug/examples/demo_model"
offline="http://127.0.0.1:9"

usage() { sed -n '2,8p' "$0" >&2; exit 2; }
[ $# -ge 2 ] || usage
command="$1"
home="$(mkdir -p "$2" && cd "$2" && pwd)"
pids="$home/demo.pids"

background() {
  local log="$1"; shift
  setsid "$@" >"$log" 2>&1 </dev/null &
  echo $! >>"$pids"
}

case "$command" in
start)
  (cd "$repo" && mise exec -- cargo build --quiet -p dotori --bin dotori --example demo_model)
  if ! pgrep -f "Xvfb :$display( |$)" >/dev/null; then
    rm -f "/tmp/.X11-unix/X$display" "/tmp/.X$display-lock"
    background "$home/xvfb.log" Xvfb ":$display" -screen 0 1280x900x24
    sleep 2
    background "$home/openbox.log" env DISPLAY=":$display" openbox
  fi
  rm -f "$home/model.port"
  background "$home/model.log" "$model" "$home/model.port"
  for _ in $(seq 50); do [ -s "$home/model.port" ] && break; sleep 0.1; done
  [ -s "$home/model.port" ] || { echo "the fake model did not start; see $home/model.log" >&2; exit 1; }
  port="$(cat "$home/model.port")"
  cat >"$home/settings.toml" <<EOF
[model]
provider = "openai-compatible"
base_url = "http://127.0.0.1:$port/v1"
model = "fake"
api_key_env = "DOTORI_DEMO_KEY"

[browser]
path = "no-browser-in-demo"
EOF
  rm -f "$home/door.json"
  background "$home/daemon.log" \
    env -u DISPLAY -u WAYLAND_DISPLAY HOME="$home" DOTORI_HOME="$home" DOTORI_DEMO_KEY=fake \
    HTTPS_PROXY="$offline" HTTP_PROXY="$offline" ALL_PROXY="$offline" NO_PROXY=127.0.0.1,localhost \
    "$dotori" daemon
  for _ in $(seq 100); do [ -s "$home/door.json" ] && break; sleep 0.1; done
  [ -s "$home/door.json" ] || { echo "the daemon did not start; see $home/daemon.log" >&2; exit 1; }
  echo "ready: $home"
  ;;
talk)
  shift 2
  if [ $# -eq 0 ]; then
    set -- "내일 오전 10시 30분에 치과 예약 있어. 아침 8시에 알려 줘" "이번 주 일정 정리해 줘" "다운로드 폴더의 영수증 PDF를 문서/영수증으로 옮겨 줘"
  fi
  {
    for message in "$@"; do
      printf '%s\n' "$message"
      sleep 3
    done
  } | env -u DISPLAY DOTORI_HOME="$home" "$dotori" chat
  ;;
shot|settings)
  [ $# -ge 3 ] || usage
  out="$3"; look="${4:-light}"; frames="${5:-1}"
  if [ "$command" = shot ]; then which=window; title='^dotori$'; else which=settings; title='^dotori .'; fi
  pkill -f "$dotori $which" 2>/dev/null || true
  background "$home/$which.log" \
    env DISPLAY=":$display" DOTORI_HOME="$home" DOTORI_THEME="$look" LANG=C.UTF-8 "$dotori" "$which"
  window=""
  for _ in $(seq 100); do
    window="$(DISPLAY=":$display" xdotool search --name "$title" 2>/dev/null | head -1 || true)"
    [ -n "$window" ] && break
    sleep 0.1
  done
  [ -n "$window" ] || { echo "no $which window; see $home/$which.log" >&2; exit 1; }
  sleep 2
  if [ "$frames" -le 1 ]; then
    DISPLAY=":$display" import -window "$window" -frame "$out"
  else
    for n in $(seq "$frames"); do
      DISPLAY=":$display" import -window "$window" -frame "${out%.png}-$n.png"
      sleep 0.05
    done
  fi
  echo "saved: $out"
  ;;
stop)
  pkill -f "$dotori window" 2>/dev/null || true
  pkill -f "$dotori settings" 2>/dev/null || true
  if [ -f "$pids" ]; then
    while read -r pid; do kill "$pid" 2>/dev/null || true; done <"$pids"
    rm -f "$pids"
  fi
  ;;
*)
  usage
  ;;
esac
