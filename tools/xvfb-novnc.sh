#!/bin/sh
# Manage a persistent Xvfb + x11vnc + noVNC session for the Linux simulator, so the SAME simulator
# instance can be watched and driven from a browser on another machine instead of a second copy.
# Usage: tools/xvfb-novnc.sh [--host ADDRESS] start|stop|status|url
#
# The three processes are separate and any of them can be restarted alone:
#   Xvfb        the display the simulator draws into (default :99)
#   x11vnc      serves that display on 127.0.0.1:5900. `-localhost` is not optional: the display has
#               no password, so binding it to anything routable would hand a live desktop to the
#               whole network, and noVNC needs it on loopback anyway.
#   websockify  serves noVNC's static page, by default on 127.0.0.1:6080. `--host` widens only this
#               one, which is the door you may want open in a browser; keep it to a trusted network.
#
# Packages, without naming a distribution: Xvfb, x11vnc, websockify and a noVNC web root
# (BUTACA_NOVNC_WEB, /usr/share/novnc by default).
set -eu

WEB_HOST=127.0.0.1
if [ "${1:-}" = --host ]; then
    [ "$#" -ge 3 ] || { echo 'Usage: $0 [--host ADDRESS] start|stop|status|url' >&2; exit 2; }
    WEB_HOST=$2
    shift 2
fi

DISPLAY_NUM=${BUTACA_XVFB_DISPLAY:-99}
WIDTH=${BUTACA_XVFB_WIDTH:-1920}
HEIGHT=${BUTACA_XVFB_HEIGHT:-1080}
DEPTH=${BUTACA_XVFB_DEPTH:-24}
VNC_PORT=${BUTACA_VNC_PORT:-5900}
WEB_PORT=${BUTACA_NOVNC_PORT:-6080}
STATE_DIR=${BUTACA_XVFB_STATE_DIR:-"${TMPDIR:-/tmp}/butaca-xvfb"}
NOVNC_WEB=${BUTACA_NOVNC_WEB:-/usr/share/novnc}
DISPLAY=":$DISPLAY_NUM"

mkdir -p "$STATE_DIR"
chmod 700 "$STATE_DIR"

pid_alive() {
    [ -f "$1" ] && kill -0 "$(cat "$1")" 2>/dev/null
}

start_one() {
    name=$1
    shift
    pidfile="$STATE_DIR/$name.pid"
    logfile="$STATE_DIR/$name.log"
    if pid_alive "$pidfile"; then
        echo "$name already running (pid $(cat "$pidfile"))"
        return
    fi
    rm -f "$pidfile"
    "$@" >>"$logfile" 2>&1 &
    pid=$!
    printf '%s\n' "$pid" >"$pidfile"
}

stop_one() {
    pidfile="$STATE_DIR/$1.pid"
    if pid_alive "$pidfile"; then
        pid=$(cat "$pidfile")
        kill "$pid" 2>/dev/null || true
        i=0
        while kill -0 "$pid" 2>/dev/null && [ "$i" -lt 30 ]; do
            sleep 0.1
            i=$((i + 1))
        done
        kill -9 "$pid" 2>/dev/null || true
        echo "stopped $1"
    fi
    rm -f "$pidfile"
}

case "${1:-}" in
    start)
        command -v Xvfb >/dev/null 2>&1 || { echo 'Xvfb is required' >&2; exit 1; }
        command -v x11vnc >/dev/null 2>&1 || { echo 'x11vnc is required' >&2; exit 1; }
        command -v websockify >/dev/null 2>&1 || { echo 'websockify is required' >&2; exit 1; }
        [ -d "$NOVNC_WEB" ] || { echo "noVNC web root not found: $NOVNC_WEB (set BUTACA_NOVNC_WEB)" >&2; exit 1; }
        if [ -f "$STATE_DIR/Xvfb.pid" ] && ! pid_alive "$STATE_DIR/Xvfb.pid"; then rm -f "$STATE_DIR/Xvfb.pid"; fi
        if ! pid_alive "$STATE_DIR/Xvfb.pid"; then
            Xvfb "$DISPLAY" -screen 0 "${WIDTH}x${HEIGHT}x${DEPTH}" -nolisten tcp -ac >>"$STATE_DIR/Xvfb.log" 2>&1 &
            echo "$!" >"$STATE_DIR/Xvfb.pid"
            i=0
            while [ ! -S "/tmp/.X11-unix/X$DISPLAY_NUM" ] && [ "$i" -lt 50 ]; do sleep 0.1; i=$((i + 1)); done
            if [ ! -S "/tmp/.X11-unix/X$DISPLAY_NUM" ]; then echo 'Xvfb failed to start; see its log' >&2; exit 1; fi
        fi
        start_one x11vnc env DISPLAY="$DISPLAY" x11vnc -display "$DISPLAY" -forever -shared -nopw -localhost -rfbport "$VNC_PORT"
        start_one websockify websockify --web "$NOVNC_WEB" "$WEB_HOST:$WEB_PORT" "127.0.0.1:$VNC_PORT"
        echo "Streaming display $DISPLAY; open http://localhost:$WEB_PORT/vnc.html?autoconnect=true&resize=remote"
        echo "Simulator environment: DISPLAY=$DISPLAY"
        ;;
    stop)
        stop_one websockify
        stop_one x11vnc
        stop_one Xvfb
        ;;
    status)
        for name in Xvfb x11vnc websockify; do
            if pid_alive "$STATE_DIR/$name.pid"; then echo "$name running (pid $(cat "$STATE_DIR/$name.pid"))"; else echo "$name stopped"; fi
        done
        ;;
    url)
        echo "http://localhost:$WEB_PORT/vnc.html?autoconnect=true&resize=remote"
        ;;
    *) echo "Usage: $0 [--host ADDRESS] start|stop|status|url" >&2; exit 2 ;;
esac
