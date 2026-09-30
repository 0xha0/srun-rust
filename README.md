# srun

A small, dependency-light client for the Srun campus-network portal
(深澜认证). Single static binary, plain ASCII output, runs on desktops and
on OpenWrt routers.

- Commands: `login`, `logout`, `status`, `switch`, `daemon` (stay online), `user`, `config`
- Defaults to BIT (`http://10.0.0.55`), any Srun server via `-s` or config
- `ac_id` auto-detected from the portal redirect; multiple users per config
- IP selection: explicit, server-detected, by interface name, or interactive
- `strict_bind` pins the connection to one interface for multi-WAN setups
- Optional TLS (`--features tls`) for https portals
- No async runtime, no threads; own output is plain ASCII, server text shown as is (`--ascii` to escape)

## Quick start

```
srun user add 1120xxxxxx        # prompts for the password, stored in the config file
srun                            # login as the default user
srun status
srun logout                     # logs out whoever is online on this connection
srun switch 1120yyyyyy          # log out, log in as another stored user, make it the default
```

`user add USERNAME` takes the portal username; `--name ALIAS` adds a short
alias, and `--user` accepts either. Passwords are stored obfuscated (see
below); `--no-password` stores no secret and the password is then taken from
`SRUN_PASSWORD` or asked for on the terminal. Adding an existing user needs
`--force`.

Without `--user`/`-u`, `login` (and `daemon`) tries the default user first
and falls back to the other stored users when the portal rejects one, with
a warning. Every command is idempotent: logging in while already online, or
logging out while offline, is a no-op with exit code 0.

Ad-hoc, without a config file:

```
srun login -u 1120xxxxxx -p PASSWORD
SRUN_PASSWORD=... srun login -u 1120xxxxxx
```

Global options go before the command: `srun -s http://10.0.0.1 -c /etc/srun/config.json login`.
Every command accepts `--help`.

## Config file

Location, first match wins: `-c PATH`, `$SRUN_CONFIG`, a `config.json`
next to the executable (portable mode: drop `srun` and `config.json` in the
same directory, e.g. `/jffs/srun/` on a router whose home is volatile), a
path baked in at build time with `SRUN_DEFAULT_CONFIG=/path cargo build`,
then `~/.config/srun/config.json` (`%APPDATA%\srun\config.json` on
Windows) and `/etc/srun/config.json`. `srun config path` prints the file in
use and the whole search order; `srun config init` writes the defaults.

```json
{
  "server": "http://10.0.0.55",
  "acid": "auto",
  "password_mode": "real",
  "retry": 3,
  "retry_delay_ms": 1000,
  "strict_bind": false,
  "double_stack": false,
  "os": "Windows 10",
  "name": "Windows",
  "n": 200,
  "type": 1,
  "default_user": "dorm",
  "daemon": { "interval": 60, "probe": "server", "logout_on_exit": false },
  "users": [
    { "name": "dorm", "username": "1120xxxxxx", "password": "obf1:3oXk...", "ifname": "eth0.2" },
    { "name": "cmcc", "username": "1120xxxxxx@cmcc", "password": "secret", "ip": "10.1.2.3" }
  ]
}
```

Passwords are stored obfuscated (`obf1:...`): XOR with a keystream derived
from the username, then base64 with a shuffled alphabet. No extra
dependencies, no key file, the binary decodes it itself. This keeps the
password out of plain sight in backups and screenshots and stops generic
base64 decoders, but it is not encryption: anyone with this program and the
file can recover it. `user add --plain` stores plaintext instead,
`user add --no-password` stores nothing, and `config obfuscate` converts the
plaintext entries of an existing file. The file is created with mode 0600.
`srun config show` prints the effective config with passwords masked.

`acid` is `"auto"` (follow the portal redirect and read `ac_id`) or a number.
`password_mode` is `real` (HMAC-MD5 of the password, what the official portal
sends) or `empty` for servers that only validate the encrypted `info` field.

## Which IP gets authorized

- nothing: the address the server sees (fine for a single-IP host)
- `-i IP`: this address
- `--ifname NAME`: the IPv4 of the interface whose name contains NAME
- `--select-ip`: pick from a numbered list of local addresses
- `--strict-bind`: also bind the socket to that address (multi-WAN / multi-dial)

Per-user `ip` / `ifname` in the config do the same thing.

`status` reports the session of the address the request comes from; the
portal cannot be asked about another address. `status -i IP` therefore
binds the request to that local address (multi-WAN hosts) and fails if the
address is not local.

The portal rejects a second authentication of the same account too soon
after the previous one (`E2532`) and too many attempts (`E2533`). `login`
and `switch` treat these as temporary and retry with a growing delay (10s,
20s, 40s); `daemon` backs off and retries the same account. If the new
account of a `switch` cannot log in, the previous account is logged back in
so the machine does not stay offline.

## Daemon mode

```
srun daemon [--interval 60] [--probe server|none|HOST:PORT] [--logout-on-exit]
```

Runs in the foreground and logs to stderr: every interval it asks the portal
whether this host is online and logs in again if not. Network errors only
back off; the daemon never switches to a fallback account over them, only
when the portal explicitly rejects the current one. A failed `ac_id`
detection (WAN still down) affects that attempt only and is retried next
round. Under procd or systemd
stderr lands in the system log; on firmware without a supervisor pipe it
through `logger` (`srun daemon 2>&1 | logger -t srun`). Failed attempts back off exponentially
(up to 10 minutes). SIGTERM / Ctrl-C stops it with exit code 0. It never
forks; let the init system keep it alive. Templates in `contrib/`:

| platform | file |
|---|---|
| OpenWrt (procd) | `contrib/openwrt/srun.init` |
| Asuswrt-Merlin (jffs scripts + cru watchdog) | `contrib/asuswrt-merlin/srun.sh` |
| systemd | `contrib/systemd/srun.service` |
| macOS launchd | `contrib/launchd/com.srun.daemon.plist` |
| Windows Task Scheduler | `contrib/windows/register-task.ps1` |

## Exit codes

| code | meaning |
|---|---|
| 0 | success (also `login --test` when already online) |
| 1 | internal error |
| 2 | usage error |
| 3 | the portal rejected the action (wrong password, arrears, already online, ...) |
| 4 | network unreachable, timeout, or unparsable reply |
| 5 | config file problem or unknown user |

Errors from the portal are printed as `rejected: wrong password (E2553)`.
The program's own output is plain ASCII. Text that comes from the portal
(product names) is shown as is; pass `--ascii` or set `SRUN_ASCII=1` on
terminals that cannot render UTF-8 (busybox, serial consoles) to get
`\uXXXX` escapes instead. Control characters and ANSI escapes from the
server are always stripped.


## Building

Stable Rust. `cargo build --release` gives an `opt-level=z`, LTO, stripped
binary. Static musl builds for routers:

```
cargo install cross
cross build --release --target aarch64-unknown-linux-musl
RUSTFLAGS="-C target-feature=+crt-static -C link-self-contained=no" \
  cross +nightly build --release --target mipsel-unknown-linux-musl -Z build-std=std,panic_abort
```

MIPS targets are Tier 3: they need nightly, `-Z build-std`, an explicit
`+crt-static`, and `link-self-contained=no` so the image's own crt objects
are used (`Cross.toml` also aliases `libgcc_eh` as the `libunwind` rustc asks
for). `mipsel` / `mips` are 32-bit soft-float (o32), which is what most
OpenWrt MIPS routers run even on 64-bit SoCs; check with `file /bin/busybox`
on the device. For `mips64el` / `mips64` add `+soft-float` to match the
image's soft-float musl (rustc marks that feature unstable; the release
workflow does this). Measured static sizes: mipsel 762 KB, mips64el 744 KB,
aarch64 610 KB.

`build/build-all.sh` builds every target it can from a unix host with
nothing but `cross` and Docker: Linux musl, MIPS, macOS (native), and Windows
via the MinGW `-gnu` targets (`x86_64-pc-windows-gnu`, `i686-pc-windows-gnu`,
static CRT). The MSVC and arm64 Windows packages come from the release
workflow's Windows runner.

On Apple Silicon, `cross` needs a Linux host toolchain and amd64 containers:

```
rustup toolchain install stable-x86_64-unknown-linux-gnu --force-non-host --profile minimal
rustup toolchain install nightly-x86_64-unknown-linux-gnu --force-non-host --profile minimal -c rust-src
DOCKER_DEFAULT_PLATFORM=linux/amd64 cross build --release --target aarch64-unknown-linux-musl
```

`cargo build --release --features tls` adds rustls (ring) with bundled roots;
`--tls-insecure` skips verification for self-signed portal certificates.
TLS is not available on the MIPS targets (ring has no MIPS backend).

Tests: `cargo test`. Protocol primitives are checked against golden vectors
generated by the MIT-licensed Go client in `ref/srun-go`
(`tools/gen_vectors`), and the CLI is exercised end to end against a mock
portal that verifies signatures the way the real one does.

## Releasing

CI (`.github/workflows/ci.yml`) runs fmt, clippy and the tests on every
push. Pushing a version tag builds every target and publishes a GitHub
Release with the packages and checksums attached:

```
git tag v0.1.0
git push origin main --tags
```

Creating the release from the GitHub UI with a new tag does the same. The
MIPS lane is best effort: if it fails the release still goes out without
those packages.

## Acknowledgements

This is a from-scratch rewrite, but it stands on two earlier clients:

- [vouv/srun](https://github.com/vouv/srun) (MIT), the Go client for BIT.
  Its `ac_id` redirect detection and portal error-code table were adopted,
  and its hash package is the oracle behind `tests/vectors.json`.
- [zu1k/srun](https://github.com/zu1k/srun) (GPL-3.0), the Rust client for
  multi-dial routers. Its feature set (config file with several users,
  interface-based IP selection, strict bind, retries) shaped this one. No
  code was copied from it; the protocol was reimplemented from the portal's
  own JavaScript.

## License

MIT.
