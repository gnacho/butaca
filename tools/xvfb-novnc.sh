#!/bin/sh
# Manage a persistent Xvfb + x11vnc + noVNC session for the Linux simulator.
# Usage: tools/xvfb-novnc.sh start|stop|status|url
set -eu

DISPLAY_NUM=${PLX_XVFB_DISPLAY:-99}
WIDTH=${PLX_XVFB_WIDTH:-1920}
HEIGHT=${PLX_XVFB_HEIGHT:-1080}
DEPTH=${PLX_XVFB_DEPTH:-24}
VNC_PORT=${PLX_VNC_PORT:-5900}
WEB_PORT=${PLX_NOVNC_PORT:-6080}
STATE_DIR=${PLX_XVFB_STATE_DIR:-"${TMPDIR:-/tmp}/plxnative-xvfb"}
NOVNC_WEB=${PLX_NOVNC_WEB:-/usr/share/novnc}
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
        command -v x11vnc >/dev/null 2>&1 || { echo 'Install x11vnc first (for example: sudo apt-get install x11vnc)' >&2; exit 1; }
        command -v websockify >/dev/null 2>&1 || { echo 'websockify is required' >&2; exit 1; }
        [ -d "$NOVNC_WEB" ] || { echo "noVNC web root not found: $NOVNC_WEB (set PLX_NOVNC_WEB)" >&2; exit 1; }
        if [ -f "$STATE_DIR/Xvfb.pid" ] && ! pid_alive "$STATE_DIR/Xvfb.pid"; then rm -f "$STATE_DIR/Xvfb.pid"; fi
        if ! pid_alive "$STATE_DIR/Xvfb.pid"; then
            Xvfb "$DISPLAY" -screen 0 "${WIDTH}x${HEIGHT}x${DEPTH}" -nolisten tcp -ac >>"$STATE_DIR/Xvfb.log" 2>&1 &
            echo "$!" >"$STATE_DIR/Xvfb.pid"
            i=0
            while [ ! -S "/tmp/.X11-unix/X$DISPLAY_NUM" ] && [ "$i" -lt 50 ]; do sleep 0.1; i=$((i + 1)); done
            if [ ! -S "/tmp/.X11-unix/X$DISPLAY_NUM" ]; then echo 'Xvfb failed to start; see its log' >&2; exit 1; fi
        fi
        start_one x11vnc env DISPLAY="$DISPLAY" x11vnc -display "$DISPLAY" -forever -shared -nopw -rfbport "$VNC_PORT"
        start_one websockify websockify --web "$NOVNC_WEB" "0.0.0.0:$WEB_PORT" "127.0.0.1:$VNC_PORT"
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
    *) echo "Usage: $0 start|stop|status|url" >&2; exit 2 ;;
esac
