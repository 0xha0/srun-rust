#!/bin/sh
# srun on Asuswrt-Merlin (RT-AX86U and friends). Everything lives under
# /jffs/srun so it survives reboots and firmware upgrades.
#
#   mkdir -p /jffs/srun
#   cp srun /jffs/srun/srun && chmod +x /jffs/srun/srun      # aarch64-unknown-linux-musl build
#   cp srun.sh /jffs/srun/srun.sh && chmod +x /jffs/srun/srun.sh
#   /jffs/srun/srun -c /jffs/srun/config.json user add USERNAME
#   /jffs/srun/srun.sh install     # boot hook + 5-minute watchdog via cru
#   /jffs/srun/srun.sh start
#
# Requires "Enable JFFS custom scripts and configs" = Yes in
# Administration > System. Logs go to syslog: tail -f /tmp/syslog.log | grep srun

DIR=/jffs/srun
BIN=$DIR/srun
CFG=$DIR/config.json
TAG=srun

# Pid of the daemon only: a concurrent "srun status"/"srun login" must not
# count as the daemon.
daemon_pid() {
    for p in $(pidof srun 2>/dev/null); do
        if tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null | grep -q ' daemon'; then
            echo $p
            return 0
        fi
    done
    return 1
}

start() {
    if daemon_pid >/dev/null; then
        echo "srun daemon already running (pid $(daemon_pid))"
        return 0
    fi
    [ -x "$BIN" ] || { echo "missing $BIN"; return 1; }
    [ -f "$CFG" ] || { echo "missing $CFG (run: $BIN -c $CFG user add USERNAME)"; return 1; }
    ( "$BIN" -c "$CFG" daemon 2>&1 | logger -t "$TAG" ) &
    sleep 1
    daemon_pid >/dev/null && echo "started (pid $(daemon_pid))" || { echo "failed to start, see syslog"; return 1; }
}

stop() {
    pid=$(daemon_pid) || { echo "not running"; return 0; }
    kill "$pid"
    echo "stopped (pid $pid)"
}

status() {
    if pid=$(daemon_pid); then
        echo "daemon: running (pid $pid)"
    else
        echo "daemon: not running"
    fi
    "$BIN" -c "$CFG" status
}

install() {
    mkdir -p /jffs/scripts
    hook=/jffs/scripts/services-start
    if [ ! -f "$hook" ]; then
        printf '#!/bin/sh\n' > "$hook"
    fi
    grep -q "$DIR/srun.sh start" "$hook" || printf '%s start\n' "$DIR/srun.sh" >> "$hook"
    chmod +x "$hook"
    cru a srun-watchdog "*/5 * * * * $DIR/srun.sh start"
    echo "installed: boot hook in $hook, watchdog cron 'srun-watchdog'"
}

uninstall() {
    stop
    cru d srun-watchdog
    [ -f /jffs/scripts/services-start ] && sed -i "\|$DIR/srun.sh start|d" /jffs/scripts/services-start
    echo "removed boot hook and watchdog"
}

case "$1" in
    start) start ;;
    stop) stop ;;
    restart) stop; start ;;
    status) status ;;
    install) install ;;
    uninstall) uninstall ;;
    *) echo "usage: $0 {start|stop|restart|status|install|uninstall}"; exit 2 ;;
esac
