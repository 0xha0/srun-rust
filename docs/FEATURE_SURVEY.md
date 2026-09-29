# srun 两个实现的 Feature 调研

> 调研日期：2026-09-29
> 对象：`srun-go`（vouv/srun v1.1.5，北理工专用）与 `srun-rust`（zu1k/srun v0.6.2，通用多校）
> 目的：为 Rust 重构（跨平台 / 嵌入式 / 最小依赖）整理候选 feature 清单

---

## 1. 一句话定位

| | srun-go | srun-rust |
|---|---|---|
| 定位 | 北理工（BIT）专用桌面 CLI，服务器 IP 写死 | 通用 srun 认证客户端，面向多拨 / 路由器场景 |
| 核心场景 | 一次 `config` 存账号，之后 `srun` 一键登录 | 命令行传参或 json 配置文件批量登录多个账号 |
| 代码量 | ~600 行 Go（不含 vendor） | ~900 行 Rust |
| 上游最后活跃 | 2022 年 | 2025 年（依赖更新为主） |

---

## 2. 协议层（两者共享的核心）

两者实现的都是同一套 srun 深澜认证流程：

1. `GET /cgi-bin/get_challenge?username=&ip=&callback=&_=`  → 拿到 `challenge`（token）和 `client_ip`/`online_ip`
2. 本地计算三个字段：
   - `info` = `{SRBX1}` + 自定义字母表 base64( xEncode( json{username,password,ip,acid,enc_ver:"srun_bx1"}, token ) )
   - `password` = `{MD5}` + HMAC-MD5(key=token, msg=?)
   - `chksum` = SHA1( token 拼接 ["", username, hmd5, acid, ip, n, type, info] )
3. `GET /cgi-bin/srun_portal?action=login&...`  → JSONP 响应，解析出 `res`/`error`/`access_token` 等
4. 登出：`GET /cgi-bin/srun_portal?action=logout&username=...`

### 2.1 协议细节上的差异（重构时必须逐条决策）

| 细节 | srun-go | srun-rust | 备注 |
|---|---|---|---|
| `password` 字段 HMAC 的明文 | **空字符串** `""` | 真实密码 | 两种都是线上存在的 srun 变种，需要抓包确认目标学校用哪种，或做成可配置 |
| `ac_id` 来源 | **自动探测**：请求 `http://10.0.0.55` 跟两次 302 跳转，从最终 URL 的 `?ac_id=` 取 | 固定默认 12，可由 `--acid` / 配置覆盖 | Go 的自动探测是真正的"零配置"体验，值得保留 |
| `callback` 参数 | `jsonp<unix秒>` | 固定 `sdu` | Rust 解析响应时硬编码 `[4..len-1]` 切掉 `sdu(` 和 `)`，脆弱 |
| 额外表单字段 | 无 | `os`、`name`、`double_stack` | 部分学校校验 os/name |
| `n` / `type` | 固定 200 / 1 | 默认 200 / 1，可配置 | |
| 时间戳 `_` | 纳秒 | 秒-2 | 无实际影响 |
| 登出参数 | 仅 username | username + ip + ac_id | |
| xEncode 实现 | int64 模拟 JS 有符号运算，靠 mask 修正 | u32 wrapping，干净 | 以 Rust 版为基础，但**两者都没有测试向量** |
| 响应错误映射 | 定义了 ~100 条 `E2531`→中文 的表，**但从未使用**（只 `Contains("Arrearage users")`） | 只 `{:#?}` 打印整个结构体 | Go 的错误码表是现成的参考资料，可直接搬 |

---

## 3. 用户可见 Feature 对比

图例：✅ 有  ❌ 无  ⚠️ 有但有缺陷  💀 代码存在但未接线/未实现

### 3.1 命令与子命令

| Feature | srun-go | srun-rust |
|---|---|---|
| `login` | ✅（且是无参默认动作） | ✅ |
| `logout` | ✅ | ✅ |
| `info` / 查询在线状态（IP、账号、钱包、套餐余额、已用流量、在线时长，调 `rad_user_info`） | ✅ | ❌ |
| `config` 交互式设置账号（密码输入关回显） | ✅ | ❌ |
| `--version` 带 OS/ARCH/Go 版本 | ✅ | ❌ |
| `--debug` 日志级别切换 | ✅ | ❌（全是 println） |
| 检查新版本（访问 GitHub releases） | 💀 `Update()` 未接到任何命令 | ❌ |
| `--continue` 持续登录 / 掉线重连守护 | ❌ | 💀 flag 存在，标注 Unimplemented |

### 3.2 账号与配置来源

| Feature | srun-go | srun-rust |
|---|---|---|
| 命令行 `-u/-p` 传账号 | ❌ | ✅ |
| 本地持久化账号文件 | ✅ `~/.srun/account.json`，base64(JSON)，**仅混淆非加密**；登录后回写 `access_token`/`acid` | ❌ |
| JSON 配置文件 | ❌ | ✅ `-c config.json`，含 server/strict_bind/double_stack/retry/n/type/acid/os/name |
| **多用户**批量登录（多拨） | ❌ | ✅ 配置文件 `users[]` 逐个登录 |
| 运营商后缀（`user@cmcc`） | ❌（无此概念） | ✅（只是 username 字符串拼接，README 有说明） |
| 认证服务器地址 | 写死 `http://10.0.0.55` | `-s` / 配置 `server` / **编译期** `AUTH_SERVER_IP` 环境变量（`build.rs` 缺它直接编译失败） |

### 3.3 IP 获取与网络绑定（Rust 独有，多拨场景关键）

| Feature | srun-go | srun-rust |
|---|---|---|
| 手动指定 `-i IP` | ❌ | ✅ |
| `-d` 从 challenge 响应 `online_ip` 自动探测 | 隐式（`ip=""` 让服务器判断） | ✅ |
| `--select-ip` 列出本机所有非回环网卡 IP 交互选择（只有一个时自动选） | ❌ | ✅ |
| 配置文件 `if_name` 按网卡名查 IPv4（子串匹配，Windows 用 GUID） | ❌ | ✅（依赖 `if-addrs`） |
| `--strict-bind` 出站 socket 绑定到指定本地 IP | ❌ | ✅（依赖 **fork 的 ureq** + `socket2`） |
| `--double-stack` | ❌ | ✅ |

### 3.4 健壮性

| Feature | srun-go | srun-rust |
|---|---|---|
| 登录失败重试（`retry_times`/`retry_delay`） | ❌ | ✅ 默认 3 次 / 1000ms |
| `--test` 登录前 tcp ping `baidu.com:80`，已联网则跳过 | ❌ | ✅ |
| 连接超时 | 1s（acid 探测）/ 默认 http client | 5s connect timeout |
| 退出码 | 失败 exit 1 | ⚠️ 重试全部失败仍返回 `Ok(())`，只打印 |
| 错误信息 | 中文短句 + 少量映射 | 结构体 Debug 打印 |

### 3.5 安全相关问题（两者都需要在重构时修）

- srun-rust 登录/登出时 `println!("login user: {:#?}", user)` **明文打印密码**（main.rs:119 / 210 / 264）
- srun-go 账号文件是 base64 而非加密，权限 0600 但目录 0755
- 两者均为 GET 传参，密码派生值出现在 URL / 服务端日志中（协议本身如此，无法避免）

---

## 4. 构建与平台

| | srun-go | srun-rust |
|---|---|---|
| 工具链 | Go 1.18，`CGO_ENABLED=0` | **nightly** + `-Z build-std` + `panic_immediate_abort`，`opt-level=z`, lto, strip, upx |
| 官方产物平台 | darwin/linux/windows 仅 amd64 | Linux musl: i686 / x86_64 / armv7hf / arm / armhf / aarch64；Windows msvc: x64 / i686 / arm64；macOS: x64 / arm64；Docker 多架构 |
| **MIPS**（大量老 OpenWrt 路由器） | Go 原生支持 `mips/mipsle`（未在 Makefile 里） | 历史上支持过（commit "cross build mips(64)(el)"），当前 CI 已移除；Rust 1.72 起 mips 降为 Tier 3，需 build-std |
| TLS | Go 标准库自带 | 可选 feature `tls`（rustls）/ `native-tls`，默认关闭以缩体积 |
| 交叉编译方式 | 纯 Go 交叉编译 | `cross` + musl.cc 工具链 |

---

## 5. 依赖盘点（嵌入式最关心）

### srun-go
- `cobra`（CLI 框架）、`logrus`（日志）、`moby/moby/pkg/term`（仅为关掉密码回显，拉进整个 moby 模块）
- 标准库覆盖 http / crypto / json

### srun-rust（默认 feature，`cargo tree` 结果）
- 直接依赖 12 个，锁定 crate 约 104 个（开 `tls` 约 102 个，rustls 替换了部分）
- 各依赖用途与可替代性：

| crate | 用途 | 体积/风险 | 可替代方案 |
|---|---|---|---|
| `ureq`（**git fork**，基于 2022 年 2.4.0） | HTTP GET | 拉入 `url` → `idna` → `icu_*` 全套 Unicode 表，是依赖树里最重的一块；fork 只为加一个 `Connector` trait 做本地地址绑定；上游早已到 3.x，fork 无人维护 | 协议只需对固定 `IP:port` 发 HTTP/1.1 GET 并读 body，可用 `std::net::TcpStream` 手写 ~100 行（srun-go 的 `core/request.go` 就是这么做的） |
| `socket2` | strict_bind 时 bind 本地地址 | 小 | 保留；或 Linux 下直接 `libc` |
| `serde` + `serde_json`（derive） | 解析响应 / 读配置 | proc-macro 编译慢、体积中等 | 响应是扁平 JSON，可手写极简解析；或保留 `serde_json` 去掉 derive 用 `Value` |
| `hmac` / `md-5` / `sha-1` | 三个摘要 | RustCrypto，no_std，很小 | 保留；或手写 md5+sha1 各 ~150 行做到零依赖 |
| `base64` | 自定义字母表 | 小 | 手写 ~30 行 |
| `if-addrs` | 枚举网卡 | 小，跨平台（libc/winapi） | 保留 |
| `getopts` | 参数解析 | 小 | 保留或手写 |
| `lazy_static` | 全局 base64 引擎 | 已过时 | `std::sync::LazyLock` |
| `quick-error` | 错误枚举 | 小 | 手写 `enum + Display` |
| `reqwest`（可选） | 备用 HTTP | 极重 | 直接删 |

结论：如果走"手写 HTTP + 手写 JSON/base64 + 保留 RustCrypto 摘要"路线，外部依赖可以压到 **2~5 个**（`if-addrs`、`socket2`、`hmac`/`md-5`/`sha-1`），且全部可在 stable 上编译。

---

## 6. 代码质量与可复用性评估

**srun-rust 可作为重构基线的部分**
- `xencode.rs`：u32 wrapping 实现干净，直接复用（需补测试向量）
- `srun.rs` 中 challenge → info/hmd5/chksum → portal 的流程与字段顺序
- IP 获取三种策略 + strict_bind 的设计思路
- 多用户配置文件 schema

**srun-go 可作为参考资料的部分**
- `core/errors.go` 的 ~100 条错误码中文表（虽然 Go 自己没用）
- `Prepare()` 跟随 302 自动取 `ac_id` 的方法
- `Info()` 调 `rad_user_info` 及字段定义 `model/response.go`
- `utils/format.go` 流量/时长格式化
- 原始 TCP 发 HTTP 请求的做法（`core/request.go`）

**两者共同的短板（重构时要一并解决）**
- 没有任何单元测试（Rust 的 3 个 `#[test]` 只是打印，无断言）
- 没有错误码 → 人类可读信息的映射真正生效
- 没有"登录后保持 / 掉线重连"的守护模式，这是嵌入式路由器场景最常见的需求
- 日志、输出格式不统一（Go 用 logrus 中文，Rust 用 Debug 打印）

---

## 7. 制定 feature 清单前需要你决策的问题

1. **目标学校**：只服务北理工（服务器 `10.0.0.55`，可以内置默认值 + acid 自动探测）还是通用（server 必须可配置）？
2. **嵌入式目标平台**具体是什么？特别是：是否需要 **MIPS**（Rust Tier 3，需 nightly build-std）？还是 armv7 / aarch64 musl 就够？Windows / macOS 还要不要？
3. **HTTP 还是 HTTPS**？目标认证服务器如果是纯 http，可以彻底不引 TLS，依赖树砍掉一半以上。
4. **`password` 字段 HMAC 空串还是真实密码**：需要抓包或试登录确认；也可以两种都实现由配置选。
5. **依赖策略**：零外部依赖（全部手写，含 md5/sha1）vs 允许少量小而稳的 crate（RustCrypto、if-addrs、socket2）。
6. **工具链策略**：stable 优先（放弃 `panic_immediate_abort` 那几 KB），还是继续 nightly 极限压体积？
7. 候选 feature 逐项取舍（建议按此表勾选）：

| 候选 feature | 来源 | 建议 |
|---|---|---|
| login / logout | 两者 | 必须 |
| info 状态查询 | Go | 建议保留，成本低 |
| 本地账号持久化（`config` 子命令） | Go | 桌面场景有用；嵌入式更适合配置文件，二选一或都要 |
| JSON 配置文件 + 多用户 | Rust | 嵌入式/多拨必须 |
| `-i` / `-d` / `--select-ip` / `if_name` 四种 IP 获取 | Rust | 建议全保留，`--select-ip` 交互在嵌入式可能用不上 |
| strict_bind | Rust | 多拨必须；单网卡场景可不要 |
| ac_id 自动探测 | Go | 建议保留，作为未配置时的兜底 |
| 重试 | Rust | 保留 |
| `--test` 登录前探测已联网 | Rust | 保留但目标地址应可配置 |
| 守护模式：定时检测掉线自动重登 | 两者都没有 | **强烈建议新增**，嵌入式核心需求 |
| 错误码中文映射 | Go（未接线） | 建议新增并真正生效 |
| double_stack / os / name / n / type 可配置 | Rust | 保留（默认值合理即可） |
| 版本检查 / 自更新 | Go（未接线） | 建议删除 |
| TLS | Rust 可选 | 视问题 3 |
| `--debug` 日志分级 | Go | 建议保留，但用极简自写 logger |
| 运营商后缀 | Rust（文档） | 零成本，保留 |
