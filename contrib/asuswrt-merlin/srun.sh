#!/bin/sh
# srun on Asuswrt-Merlin (RT-AX86U, RT-AC86U, ...). Everything lives in
# /jffs/srun so it survives reboots and firmware upgrades.
#
#   mkdir -p /jffs/srun
#   cp srun /jffs/srun/srun && chmod +x /jffs/srun/srun      # aarch64-unknown-linux-musl build
#   cp srun.sh /jffs/srun/srun.sh && chmod +x /jffs/srun/srun.sh
#   /jffs/srun/srun -c /jffs/srun/config.json user add USERNAME
#   /jffs/srun/srun.sh install     # boot + WAN hooks, 5-minute watchdog
#   /jffs/srun/srun.sh start
#
# Needs "Enable JFFS custom scripts and configs" = Yes (Administration > System).
# Logs: tail -f /tmp/syslog.log | grep srun
#
# install adds one line to /jffs/scripts/services-start (start at boot) and
# /jffs/scripts/wan-event (on "connected": restart the daemon so it checks
# right away instead of waiting out a backoff, then nudge ntpd). cru jobs
# live in RAM, so start re-adds the watchdog every time.

DIR=/jffs/srun
HOOKS=/jffs/scripts
BIN=$DIR/srun
CFG=$DIR/config.json
TAG=srun

# Pid of the daemon only; a concurrent "srun status" must not count.
daemon_pid() {
    for p in $(pidof srun 2>/dev/null); do
        if tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null | grep -q ' daemon'; then
            echo $p
            return 0
        fi
    done
    return 1
}

watchdog() {
    cru a srun-watchdog "*/5 * * * * $DIR/srun.sh start"
}

start() {
    watchdog
    if pid=$(daemon_pid); then
        echo "srun daemon already running (pid $pid)"
        return 0
    fi
    [ -x "$BIN" ] || { echo "missing $BIN"; return 1; }
    [ -f "$CFG" ] || { echo "missing $CFG (run: $BIN -c $CFG user add USERNAME)"; return 1; }
    # Ignore SIGHUP so the daemon outlives the ssh session that started it.
    ( trap '' HUP; "$BIN" -c "$CFG" daemon 2>&1 | logger -t "$TAG" ) </dev/null >/dev/null 2>&1 &
    sleep 1
    if pid=$(daemon_pid); then
        echo "started (pid $pid)"
    else
        echo "failed to start, see syslog"
        return 1
    fi
}

# Wait for the daemon to exit (it finishes the request in flight first).
stop() {
    pid=$(daemon_pid) || { echo "not running"; return 0; }
    kill "$pid"
    i=0
    while [ $i -lt 10 ] && kill -0 "$pid" 2>/dev/null; do
        sleep 1
        i=$((i + 1))
    done
    if kill -0 "$pid" 2>/dev/null; then
        kill -9 "$pid"
        echo "killed (pid $pid, no exit after 10s)"
    else
        echo "stopped (pid $pid)"
    fi
}

# Merlin's ntpd gives up for a long time when it ran before the portal login;
# once online, restart it if the clock is still unset.
ntp_nudge() {
    i=0
    while [ $i -lt 60 ]; do
        [ "$(nvram get ntp_ready 2>/dev/null)" = 1 ] && return 0
        # nvram is cheap; ask the portal only every 10s
        if [ $((i % 10)) -eq 0 ] && "$BIN" -c "$CFG" -q status 2>/dev/null | grep -q '^online: yes'; then
            service restart_ntpd >/dev/null 2>&1
            logger -t "$TAG" "online, clock not synced yet: restarted ntpd"
            return 0
        fi
        sleep 2
        i=$((i + 2))
    done
}

# wan-event UNIT EVENT: act on "connected" only, in the background.
wan_event() {
    [ "$2" = connected ] || return 0
    (
        logger -t "$TAG" "WAN $1 connected: restarting the daemon"
        stop >/dev/null
        start >/dev/null
        ntp_nudge
    ) </dev/null >/dev/null 2>&1 &
}

status() {
    if pid=$(daemon_pid); then
        echo "daemon: running (pid $pid)"
    else
        echo "daemon: not running"
    fi
    if cru l 2>/dev/null | grep -q '#srun-watchdog#'; then
        echo "watchdog: cron srun-watchdog"
    else
        echo "watchdog: missing (srun.sh start adds it)"
    fi
    "$BIN" -c "$CFG" status
}

# add_hook FILE LINE: append LINE once, creating the script if needed.
add_hook() {
    [ -f "$1" ] || printf '#!/bin/sh\n' > "$1"
    grep -qF "$2" "$1" || printf '%s\n' "$2" >> "$1"
    chmod +x "$1"
}

install() {
    mkdir -p "$HOOKS"
    add_hook "$HOOKS/services-start" "$DIR/srun.sh start"
    add_hook "$HOOKS/wan-event" "$DIR/srun.sh wan-event \"\$@\""
    watchdog
    echo "installed: $HOOKS/services-start and $HOOKS/wan-event hooks, watchdog cron 'srun-watchdog'"
}

uninstall() {
    stop
    cru d srun-watchdog
    for h in services-start wan-event; do
        [ -f "$HOOKS/$h" ] && sed -i "\|$DIR/srun.sh |d" "$HOOKS/$h"
    done
    echo "removed hooks and watchdog"
}

case "$1" in
    start) start ;;
    stop) stop ;;
    restart) stop; start ;;
    status) status ;;
    wan-event) shift; wan_event "$@" ;;
    install) install ;;
    uninstall) uninstall ;;
    *) echo "usage: $0 {start|stop|restart|status|wan-event UNIT EVENT|install|uninstall}"; exit 2 ;;
esac
