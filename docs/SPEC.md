# srun Rust 重构：需求与预期设计

> 日期：2026-09-29
> 状态：已确认（2026-09-29 第二轮决策已合并，见第 11 节）
> 前置文档：`FEATURE_SURVEY.md`（两个旧实现的 feature 调研）

---

## 1. 已确定的决策

| # | 议题 | 决策 |
|---|---|---|
| 1 | 服务对象 | 通用 srun 客户端，**默认配置指向北理工**（`http://10.0.0.55`），一切可覆盖 |
| 2 | 平台 | 全平台：Linux（含嵌入式 musl 静态）、Windows、macOS，尽量覆盖 MIPS |
| 3 | 传输 | 默认 http；https 作为编译期 feature 提供，默认关闭 |
| 4 | `password` 字段 HMAC 明文 | 可配置 `password_mode`，默认 `real`（门户 JS 实测确认），见 6.2 |
| 5 | 依赖 | 允许少量主流、维护良好的 crate；HTTP、JSON、base64、参数解析、日志全部手写 |
| 6 | 工具链 | **stable 优先**；MIPS 单独走 nightly 通道，尽力支持 |
| 7 | 用户管理 | CLI 可增删改用户；配置文件位置可指定，不写死 |
| 8 | 守护模式 | 要，作为前台常驻进程实现，进程托管交给平台（见第 7 节） |
| 9 | 输出 | 程序自身产出的文字仅限 ASCII，不使用颜色、ANSI 转义、Unicode 符号；服务器返回的文本默认原样尽力显示，`--ascii` / `SRUN_ASCII=1` 时转义为 `\uXXXX`（busybox 终端用） |
| 10 | 版本检查 / 自动更新 | 彻底删除 |
| 11 | 许可证 | 旧 Rust 版为 GPL-3，**不直接复制其代码**，算法按协议重写；旧 Go 版为 MIT，错误码表等可直接采用 |
| 12 | 配置格式 | JSON，用 `serde` + `serde_json`（derive）；运行时 5 个 crate，体积约 +50~120 KB |
| 13 | MIPS | 四个 target 都做，主 `mips64el/mips64-unknown-linux-muslabi64`，次 `mipsel/mips-unknown-linux-musl`；nightly 通道，CI 允许失败 |
| 14 | 仓库布局 | `srun-dev/` 根目录即新 crate，旧实现移至 `ref/`（gitignore），文档移至 `docs/` |
| 15 | 命令名 | `status` 为主，`info` 为别名 |
| 16 | 密码存储 | 混淆而非加密：`obf1:` + 用户名派生的 HMAC-MD5 XOR 流 + 打乱字母表 base64。零新增依赖、自包含、无密钥文件；边界是拿到程序和文件的人仍可还原。`--plain` 可存明文，`--no-password` 可不存 |

---

## 2. 目标与非目标

### 目标
- 单个静态二进制，无运行时依赖，在 OpenWrt 一类的 busybox 环境可直接运行
- 无 TLS 的 musl 静态构建体积目标：**小于 1 MB**（争取 500 KB 上下，upx 后更小）
- 单线程、阻塞 IO、无 async runtime，内存占用以 KB 计
- 协议层有可断言的单元测试，用旧 Go 实现作为 oracle 生成测试向量
- 一份配置文件、一套命令行，桌面和路由器用法一致

### 非目标
- 图形界面、系统托盘
- 自更新、版本检查
- 兼容旧 Go 版的 `~/.srun/account.json`
- 自行 fork/setsid 变成后台进程（交给 procd / systemd / launchd）
- 完整 HTTP 客户端（只实现 srun 协议需要的 GET + 读 body）

---

## 3. Feature 清单（最终）

优先级：P0 第一个可用版本必须有；P1 正式发布前完成；P2 之后再做。

| Feature | 优先级 | 来源 | 说明 |
|---|---|---|---|
| `login` | P0 | 两者 | 无子命令时的默认动作 |
| `logout` | P0 | 两者 | |
| `status`（别名 `info`）在线状态查询 | P0 | Go | 调 `rad_user_info`，输出 IP / 账号 / 钱包 / 余额 / 流量 / 时长 |
| 配置文件 + 多用户 | P0 | Rust | 一个文件，多个 `[user]` 段 |
| CLI 管理用户：`user add/remove/list/default` | P0 | 新 | 密码可交互输入（关回显）或从 stdin/环境变量读 |
| 配置文件路径可指定 | P0 | 新 | `-c` > `$SRUN_CONFIG` > 平台默认路径，见 5.2 |
| `ac_id` 自动探测 | P0 | Go | 未配置时跟随 302 取参数；探测失败回落默认 12 |
| 四种 IP 获取：`-i` / `--detect-ip` / `--select-ip` / `ifname` | P0 | Rust | `--select-ip` 是交互式，嵌入式一般用 `ifname` |
| `--strict-bind` 出站 socket 绑定本地 IP | P0 | Rust | 多拨必需 |
| 登录重试 `retry` / `retry_delay_ms` | P0 | Rust | |
| 错误码到 ASCII 英文短句的映射 | P0 | Go（未接线） | 翻译 Go 的中文表；未知码原样打印 |
| 守护模式 `daemon` | P1 | 新 | 见第 7 节 |
| `--test` 登录前探测已联网 | P1 | Rust | 探测目标可配置，默认 `auth server` 本身的 `rad_user_info` |
| `double_stack` / `os` / `name` / `n` / `type` 可配置 | P1 | Rust | 有合理默认值 |
| 运营商后缀 `user@cmcc` | P1 | Rust | 纯字符串，零成本 |
| `password_mode` 可配置 | P1 | 新 | 见 6.2 |
| 日志分级 `-v` / `-q` | P1 | Go | 手写 logger |
| TLS（feature `tls`） | P1 | Rust | 见 6.4 |
| 平台服务模板：procd / systemd / launchd | P1 | 新 | 放在 `contrib/` |
| `config show` / `config path` / `config init` | P2 | 新 | 打印生效配置、路径、写模板 |
| 版本检查 / 自更新 | 删除 | Go | |

---

## 4. 命令行设计

```
srun [GLOBAL] [COMMAND] [OPTIONS]

COMMAND
  login      authenticate (default when no command given)
  logout     de-authenticate
  status     show online info (alias: info)
  daemon     stay online: check and re-login in a loop (foreground)
  user       manage users in config: add | remove | list | default
  config     show | path | init
  version    print version and target triple

GLOBAL
  -c, --config PATH      config file (default: see config path rules)
  -s, --server URL       auth server, e.g. http://10.0.0.55
  -v                     more verbose (repeatable: -vv)
  -q                     quiet, errors only
      --acid N           override ac_id (default: auto)
      --tls-insecure     skip certificate verification (tls feature only)
  -h, --help
```

登录 / 登出的用户选择：

```
  -u, --username NAME    ad-hoc user (with -p)
  -p, --password PASS    ad-hoc password (prefer SRUN_PASSWORD env or config file)
      --user NAME        pick a named user from config
      --all              act on every user in config (multi-dial)
  (none)                 use the default user in config
```

IP 相关（login / logout / daemon 共用）：

```
  -i, --ip IP            use this IP
  -d, --detect-ip        let the server tell us (online_ip)
      --select-ip        interactive: list NIC IPs and choose
      --ifname NAME      pick the IPv4 of this interface (substring match)
      --strict-bind      bind outgoing socket to the chosen IP
```

登录参数：

```
      --retry N              default 3
      --retry-delay MS       default 1000
      --test [--probe MODE]  skip login if already online
      --double-stack
      --os STR / --name STR / --n N / --type N
      --password-mode real|empty|auto
```

守护模式：

```
  srun daemon [--interval SECS] [--probe server|HOST:PORT|none] [--logout-on-exit]
```

用户管理：

```
  srun user add NAME [-u USERNAME] [--ip IP | --ifname NAME] [--password-stdin]
  srun user remove NAME
  srun user list
  srun user default NAME
```

`user add` 不带 `--password-stdin` 时交互提示输入密码，关回显（unix 用 termios，windows 用 SetConsoleMode）。

### 退出码

| 码 | 含义 |
|---|---|
| 0 | 成功（含 `--test` 发现已在线） |
| 1 | 内部错误 / 未分类 |
| 2 | 命令行用法错误 |
| 3 | 认证被服务器拒绝（密码错误、欠费、已在线等） |
| 4 | 网络不可达 / 超时 / 响应无法解析 |
| 5 | 配置文件错误 / 找不到用户 |

---

## 5. 配置文件

### 5.1 格式

JSON，由 `serde_json` 读写。CLI 修改用户时整文件按固定格式（2 空格缩进）重写。

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
  "default_user": "dorm",
  "daemon": { "interval": 60, "probe": "server", "logout_on_exit": false },
  "users": [
    { "name": "dorm", "username": "1120xxxxxx", "password": "secret", "ifname": "eth0.2" },
    { "name": "cmcc", "username": "1120xxxxxx@cmcc", "password": "secret", "ip": "10.1.2.3" }
  ]
}
```

### 5.2 路径解析顺序

1. `-c PATH`
2. 环境变量 `SRUN_CONFIG`
3. Linux / macOS：`$XDG_CONFIG_HOME/srun/config.json`，否则 `~/.config/srun/config.json`；若不存在再找 `/etc/srun/config.json`
4. Windows：`%APPDATA%\srun\config.json`

`config path` 打印最终选中的路径。写入时创建目录，文件权限 0600（unix）。

### 5.3 密码存储

明文存储，文件权限 0600。旧 Go 版的 base64 只是混淆，不再假装加密。嵌入式平台没有 keychain 可用，桌面平台如果需要，可以不写 `password` 字段，改为运行时从 `SRUN_PASSWORD` 环境变量或 `--password-stdin` 提供。

---

## 6. 协议层设计

### 6.1 流程

与两个旧实现一致：`get_challenge` → 计算 `info` / `password` / `chksum` → `srun_portal action=login`。登出走 `srun_portal action=logout`，状态走 `rad_user_info`。

### 6.2 `password_mode`

- `real`（默认）：`{MD5}` + HMAC-MD5(key=token, msg=真实密码)。2026-09-29 抓取北理工门户 `jquery.srun.portal.js` 确认官方前端即为 `md5(password, token)`
- `empty`：HMAC-MD5(key=token, msg="")。旧 Go 版行为，服务器对 `password` 字段宽松时可用，保留为兼容选项

`info` 字段按官方前端格式生成：`JSON.stringify({username,password,ip,acid,enc_ver})`，键按声明顺序、紧凑、`acid` 为字符串。

### 6.3 `ac_id` 探测

`acid = auto` 时：GET `server/`，跟随最多 3 次 302，从最终 `Location` 的 `ac_id` 参数取值；任一步失败回落 12 并打 warning。探测结果不缓存（成本是一次请求）。

### 6.4 HTTP 与 TLS

- 手写 HTTP/1.1 客户端：仅支持 GET，`Connection: close`，读到 EOF；解析状态行、`Location`、`Content-Length` / chunked；不做重定向跟随以外的任何事
- URL 解析只处理 `scheme://host[:port]/path`，host 可为 IPv4 / IPv6 字面量 / 域名
- JSONP 解析：找第一个 `(` 和最后一个 `)`，不依赖 callback 名
- 连接超时默认 5s，读超时默认 10s
- `strict_bind`：用 `socket2` 先 `bind` 再 `connect`
- TLS：feature `tls`，`rustls` + `ring` 后端 + `webpki-roots` 内置根证书（不依赖系统证书库）；`--tls-insecure` 跳过校验（校园网自签证书常见）。`ring` 不支持 MIPS，MIPS 通道不提供 TLS

### 6.5 错误码映射

采用旧 Go 版 `core/errors.go` 的表，翻译为 ASCII 英文短句。输出形如：

```
error: login rejected: E2553 wrong password
```

未知码：`error: login rejected: E9999 (unknown code)`，并在 `-v` 下打印原始响应。

---

## 7. 守护模式与嵌入式部署

### 7.1 行为

`srun daemon` 是前台进程，不 fork、不写 pidfile、不改 cwd：

```
loop:
  online = probe()
  if not online:
     login (with retry)
  sleep interval (default 60s; login 连续失败时指数退避到最多 10 分钟)
on SIGTERM/SIGINT (unix) / Ctrl-C (windows):
  if --logout-on-exit: logout
  exit 0
```

`probe` 策略：
- `server`（默认）：请求 `rad_user_info`，服务器说在线即在线。只依赖认证服务器，校园网内一定可达
- `HOST:PORT`：TCP 连接探测，适合校验出网
- `none`：不探测，每个周期无条件登录（有些服务器对重复登录返回 `E2620 already online`，视为成功）

### 7.2 进程托管由平台负责

| 平台 | 方式 | 我们提供 |
|---|---|---|
| OpenWrt | procd | `contrib/openwrt/etc/init.d/srun`（`respawn`，日志走 logd） |
| 通用 Linux | systemd | `contrib/systemd/srun.service`（`Restart=always`） |
| macOS | launchd | `contrib/launchd/com.srun.daemon.plist`（`KeepAlive`） |
| Windows | 任务计划程序 / NSSM | `contrib/windows/register-task.ps1`（登录时启动） |

这是嵌入式常规做法：二进制只管前台跑，崩溃或退出由 init 系统拉起。

### 7.3 嵌入式约束

- 时钟可能是 1970：协议里的 `_` 时间戳只是防缓存，不影响；日志时间戳可用 `--no-timestamp` 关掉
- 无 `/etc/resolv.conf` 或 DNS 不通：默认服务器是 IP，`probe = server` 不依赖 DNS
- 只读根文件系统：配置放 `/etc/srun/config`，运行时不写任何文件
- 内存：单线程，最大分配量是一次 HTTP 响应体（几 KB）

---

## 8. 输出与日志规范

- 全部输出限 ASCII 可打印字符加换行；禁止颜色、ANSI 转义、Unicode 符号、emoji、制表符对齐
- 一行一条，`key: value` 或 `LEVEL message key=value` 形式，便于 grep
- 正常输出到 stdout，日志和错误到 stderr
- 默认级别 info；`-v` 打印请求 URL（密码派生字段脱敏）；`-vv` 打印原始响应；`-q` 只打错误
- 任何级别都**不打印密码明文**

示例：

```
$ srun status
online: yes
ip: 10.27.196.218
username: 1120xxxxxx
wallet: 12.34
balance: 0.00
used: 1.23 GB
uptime: 01:02:03

$ srun login
info: acid detected: 1
info: challenge ok ip=10.27.196.218
info: login ok user=1120xxxxxx
```

---

## 9. 依赖清单与体积

### 默认构建

| crate | 用途 | 说明 |
|---|---|---|
| `serde`(derive), `serde_json` | 配置文件与协议响应 | 运行时 serde/serde_json/itoa/ryu/memchr，编译期 serde_derive/syn/quote/proc-macro2 |
| `hmac`, `md-5`, `sha1` | 三个摘要 | RustCrypto，no_std，稳定 |
| `if-addrs` | 枚举网卡 IP | 跨平台，Windows 下拉 `windows-sys` |
| `socket2` | bind 后 connect | rust-lang 官方 |
| `libc`（unix）/ `windows-sys`（windows） | 关回显、信号 | 各平台只编译一个 |

预计运行时链接 crate 约 12 个。

### `tls` feature 额外

`rustls`（`ring` 后端）、`webpki-roots`，约 20 到 30 个 crate。

### 手写部分

HTTP/1.1 GET 客户端、极简 URL 解析、自定义字母表 base64、xEncode、命令行参数解析、logger。

### 编译配置（stable）

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

不使用 `-Z build-std` 和 `panic_immediate_abort`，这两项只在 MIPS 通道上用。

---

## 10. 平台矩阵与工具链

| 目标 | 工具链 | 构建方式 | 备注 |
|---|---|---|---|
| x86_64 / aarch64 / armv7hf / arm / i686 `-unknown-linux-musl` | stable | `cross` 或 rustup target + musl 工具链 | 主力嵌入式目标，静态链接 |
| x86_64 / aarch64 `-unknown-linux-gnu` | stable | 原生 | 桌面 Linux |
| x86_64 / aarch64 / i686 `-pc-windows-msvc` | stable | 原生 | `+crt-static` |
| x86_64 / aarch64 `-apple-darwin` | stable | 原生 | |
| `mips64el/mips64-unknown-linux-muslabi64`（主）、`mipsel/mips-unknown-linux-musl`（次，o32 软浮点） | **nightly** + `-Z build-std` | `cross` | Tier 3，尽力支持，CI 失败不阻塞发布，无 TLS；上板前用 `file /bin/busybox` 确认用户态位数 |

CI：
- `cargo fmt --check`、`cargo clippy -D warnings`、`cargo test` 在 Linux / macOS / Windows 三个 runner 上跑
- release tag 触发全矩阵交叉构建，产物带 sha256
- MIPS 作为 `continue-on-error` 的独立 job

---

## 11. 决策记录（2026-09-29 第二轮）

| 问题 | 结论 |
|---|---|
| `password_mode` 默认值 | `real`。抓取门户 JS 确认 `hmd5 = md5(password, token)` |
| 配置文件格式 | JSON + serde/serde_json（derive），扩展成本低 |
| MIPS 目标 | 四个都做，主 64 位 |
| TLS 后端 | `rustls` + `ring` + `webpki-roots`，MIPS 通道不带 TLS |
| 命令名 | `status` 主、`info` 别名 |
| 项目位置与许可证 | 根目录新建，MIT；旧实现移入 `ref/` 仅作参考 |

### 校内实测记录（2026-09-29，10.0.0.55）

- `GET /` → 302 `/index_1.html` → 302 `/srun_portal_pc?ac_id=8&theme=bit`，当日 `ac_id = 8`
- 服务器版本 `SRunCGIAuthIntfSvr V1.18 B20220802`
- 门户 JS 常量 `n=200, type=1, enc="srun_bx1"`；`ip = data.ip || response.client_ip`
- `rad_user_info` 在线时 `error == "ok"`；含 `ServerFlag=4294967040`、`sum_bytes≈2.2e11`、中文 `products_name`；`ecode` 时而数字时而字符串
- https 不可达

## 12. 里程碑（粗略，确认第 11 节后细化）

| 里程碑 | 内容 | 可验证结果 |
|---|---|---|
| M0 基线 | 新 crate 骨架、profile、CI 矩阵（stable 全平台）、logger、参数解析 | 全平台编译出空壳二进制，体积基线 |
| M1 协议 | xEncode / base64 / HMAC / chksum / JSON / HTTP 客户端，用 Go 实现生成测试向量 | `cargo test` 全绿；mock server 上跑通 login |
| M2 命令 | `login` / `logout` / `status`，配置文件读取，四种 IP 获取，`strict_bind`，acid 探测，错误码映射 | 校内实测登录成功；确定 `password_mode` |
| M3 用户管理 | `user add/remove/list/default`，`config` 子命令，关回显输入 | 桌面平台完整流程 |
| M4 守护 | `daemon` 循环、信号处理、`contrib/` 模板 | OpenWrt 上 procd 托管跑过一晚 |
| M5 TLS 与 MIPS | `tls` feature、nightly MIPS 通道 | 对 https 测试服务器登录；MIPS 二进制在目标板运行 |
| M6 发布 | README、release workflow、体积检查 | tag 触发产物 |

## 13. 实现与验证记录（2026-09-29）

代码已按第 12 节里程碑落地在仓库根目录（commit 历史 M0 → M4）。

| 项目 | 结果 |
|---|---|
| 单元 + 集成测试 | 32 单元、10 协议客户端（mock 服务器）、7 CLI 端到端（含守护模式踢线重登与 SIGTERM 退出）全部通过 |
| 协议原语 | xEncode / 自定义 base64 / HMAC-MD5 / SHA1 与 `ref/srun-go` 生成的 8 组 golden 向量逐字节一致 |
| 校内实测 `status` | 10.0.0.55 返回正确，输出经 `grep` 确认纯 ASCII，中文套餐名以 `\uXXXX` 呈现 |
| 校内实测 `login` | acid 自动探测得 8；用假账号发起登录，服务器回 `ip_already_online_error`（本机已在线），说明请求格式与签名被接受。**真实账号在离线状态下的登录尚未验证**，需要用户执行 `srun login -u 学号 -p 密码 -v` |
| `logout` | 未在真实服务器执行，避免踢掉当前会话 |
| TLS | `--features tls` 编译通过；对公网 https 站点握手成功；本机 openssl 自签服务：默认拒绝证书，`--tls-insecure` 接受 |
| 交叉类型检查 | `cargo check --target x86_64-pc-windows-msvc` 与 `x86_64-unknown-linux-musl` 通过（验证 windows-sys / libc 分支） |
| 体积 | aarch64-apple-darwin release：约 476 KB（无 TLS）、约 1.14 MB（含 TLS） |
| 依赖 | 默认 feature 运行时链接约 30 个 crate（含 serde 系、RustCrypto、if-addrs、socket2、libc 及其传递依赖） |

待办：
- 真实账号离线登录验证（用户）
- OpenWrt 实机运行与 procd 托管（用户路由器）
- MIPS 本机已验证（cross 0.2.5，Apple Silicon 上 `DOCKER_DEFAULT_PLATFORM=linux/amd64`）：mipsel 静态 762 KB，mips64el 静态 744 KB
- MIPS 静态链接的完整配方：`RUSTFLAGS="-C target-feature=+crt-static -C link-self-contained=no"` + `Cross.toml` 的 pre-build 把 `libgcc_eh.a` 别名为 `libunwind.a`。不加 `crt-static` 产物是依赖 `/lib/ld-musl-*.so.1` 的动态 PIE；加了但不关 self-contained 会因 Tier 3 没有自带 crt/libunwind 而链接失败
- mips64el/mips64：cross 镜像工具链是软浮点，Rust 目标是硬浮点，用 `+soft-float` 对齐后链接干净；rustc 已警告该 feature 不稳定、未来会变硬错误。备选是去掉 `+soft-float` 直接链接（有 ABI 警告，本程序没有浮点跨 FFI 边界，理论上可用），需实机验证
- 在 Apple Silicon 上 `CROSS_CONTAINER_OPTS="--platform linux/amd64"` 只对 `docker run` 生效，pre-build 走 `docker build` 需要 `DOCKER_DEFAULT_PLATFORM`
- Apple Silicon 本机用 cross 需要 `--force-non-host` 装 stable 与 nightly 的 x86_64-unknown-linux-gnu 工具链，并设置 `DOCKER_DEFAULT_PLATFORM=linux/amd64`
