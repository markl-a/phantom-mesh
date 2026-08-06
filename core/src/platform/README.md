# `core/src/platform/` — 各作業系統平台轉接器（per-OS platform adapters）

此目錄存放各作業系統（per-operating-system，依各 OS 區分）的程式碼，讓
`spectyn-mesh` 的其餘部分得以維持 OS-agnostic（與作業系統無關）。凡是在
macOS、Linux、Windows、Android 與 iOS 之間有差異的部分 — 子行程（child
process，由程式衍生的下層行程）如何被衍生、如何被 sandbox（沙箱，受限執行
環境）隔離、RAM / CPU / OS 字串如何讀取、設定檔放在哪裡、release（發行）
binary（二進位執行檔）叫什麼名字 — 全都藏在單一 trait（特徵介面）之後，因此
呼叫端（callers）永遠不必自己寫 `#[cfg(target_os = ...)]`。

## 此抽象層提供什麼

合約（contract）就是 [`mod.rs`](./mod.rs) 裡的 `PlatformAdapter` trait。每個
受支援的 target（編譯目標）都實作它：

| Method | 用途 |
| --- | --- |
| `make_command(program, args, raw_cmd)` | 為一次工具呼叫建立 `tokio::process::Command`，並套用該 OS 的 sandbox。 |
| `shell_command(cmd)` | 為含有管線/重導向（pipes/redirects）的字串建立命令（Unix 上用 `sh -c`、Windows 上用 `cmd.exe /C`），套用相同的 sandbox 規則。 |
| `ram_mb()` | 系統總記憶體（RAM），單位 MB；盡力而為（best-effort），失敗時回傳 `0`。 |
| `cpu_name()` | CPU 型號字串；盡力而為，失敗時回傳 `"Unknown CPU"`。 |
| `os_name()` | 人類可讀的 OS 字串。 |
| `dist_binary_name()` | 此 target 的 release 產物（release-artifact）檔名。 |
| `config_dir()` | 使用者層級（user-level）的設定/資料目錄。 |

`mod.rs` 中還有兩項額外項目與此 trait 並列：

- **自由函式包裝器（Free-function wrappers）**（`platform::make_command(...)`、
  `platform::ram_mb()` 等）— 對 `current()` 的輕量委派者（delegators），
  保留原本的扁平 API（flat API），讓既有呼叫端無需任何修改。
- **`mdns_advertise(...)`** — 一個 `async`（非同步）自由函式（刻意不放進
  trait，好讓 trait 維持同步、避免 `async-trait`）。它自行做內聯
  （inline，行內）`#[cfg]` 分派：macOS 上用 `dns-sd`、Linux 上用
  `avahi-publish-service`、Windows 上不支援（改用 coordinator（協調者）），
  其他 target 亦不支援。

## `mod.rs` 如何選擇實作

1. 每個 target 的模組都採條件式編譯（conditionally compiled）：
   ```rust
   #[cfg(target_os = "android")] mod android;
   #[cfg(target_os = "ios")]     mod ios;
   #[cfg(target_os = "linux")]   mod linux;
   #[cfg(target_os = "macos")]   mod macos;
   #[cfg(target_os = "windows")] mod windows;
   ```
   只有與建置 target 相符的模組會被編入。
2. `current() -> &'static dyn PlatformAdapter` 透過相符的
   `#[cfg(target_os = ...)]` 分支（arm），回傳該模組的 `PLATFORM` static。
3. 最後一個 `#[cfg(not(any(...)))]` 分支會呼叫 `compile_error!`，因此為未
   列出的 OS 建置時會明確失敗（fails loudly），並附上訊息告訴貢獻者
   （contributor）去新增一個模組並接進 `current()`。

每個各 OS 模組都遵循相同的形狀：

```rust
pub struct Platform;
pub static PLATFORM: Platform = Platform;
impl PlatformAdapter for Platform { /* ... */ }
```

選擇完全在編譯期（compile time）解析；除了 `&'static dyn` 的間接層
（indirection）外，沒有執行期（runtime）OS 偵測，也沒有動態分派
（dynamic dispatch）成本。

## 各 OS 模組

### [`macos.rs`](./macos.rs)
透過 `crate::process_sandbox::macos::wrap`，以 Seatbelt（`sandbox-exec`）
包裹每個被衍生的工具。透過 `sysctl`（`hw.memsize`、
`machdep.cpu.brand_string`）讀取 RAM/CPU，並透過 `sw_vers` 讀取 OS 版本。
設定目錄：`~/Library/Application Support/ai.spectynmesh.app`。
Dist binary：`spectyn-macos-arm64`。

### [`linux.rs`](./linux.rs)
透過 `crate::process_sandbox::linux::install_pre_exec`，安裝一個 Landlock
LSM（Linux Security Module，Linux 安全模組）pre-exec hook（exec 前掛鉤，
等同於 Linux 上的 Seatbelt）。讀取 `/proc/meminfo`、`/proc/cpuinfo` 與
`/etc/os-release` 以取得系統資訊。設定目錄：`~/.spectyn-mesh`。Dist
binary：`spectyn-linux-arm64`。它也存放標準的 journald socket 常數
（`JOURNAL_SOCKETS`）以及 systemd 日誌路徑所用的
`journal_routing_available()` 輔助函式（helper）。

### [`windows.rs`](./windows.rs)
對於需要 shell 功能的工具，會經由 `cmd.exe /C` 轉送；但對已知的跨平台
（cross-platform）binary（`cargo`、`git`、`node`、`python`、…；見
`is_cross_platform_bin`）則直接衍生，因此其 argv（引數向量）會原封不動傳遞。
透過 `wmic` 讀取 RAM/CPU。設定目錄：`%APPDATA%\spectyn-mesh`。Dist
binary：`spectyn-windows-x86_64.exe`。無行程層級（process-level）sandbox、
也無 mDNS（多點傳播 DNS）。

### [`android.rs`](./android.rs)
一個輕量墊片（shim，相容轉接層）：無 sandbox（Landlock 僅限 Linux；
Termux/APK 環境本身已運行於 app sandbox 之中），也不做 `/proc` 解析，因此
`ram_mb()`、`cpu_name()` 與 `os_name()` 會回傳未抽取（pre-extraction）前的
預設值（`0`、`"Unknown CPU"`、`"Unknown OS"`）。設定目錄：`~/.spectyn-mesh`。
Dist binary：`spectyn-aarch64-linux-android`。Termux 偵測、app-container
（應用程式容器）路徑對應，以及前景服務（foreground-service）生命週期，皆屬
未來工作（epic P-AND-2）。

### [`ios.rs`](./ios.rs)
一個刻意極簡的 stub（樁，最小佔位實作），之所以必要，是因為 Tauri app
（`spectyn-mesh-app`）在每個 target 上都把 core lib 連結為相依套件
（dependency），包含 `aarch64-apple-ios{,-sim}`。若 `current()` 中沒有
iOS 分支，`compile_error!` 分支就會觸發並破壞 `scripts/package-ios.sh`。
它重用 macOS 風格的 `sysctl` 自省（introspection，盡力而為，在 App Store
sandbox 規則下進行），並使用相同的 Application Support 設定目錄。無行程層級
sandbox — iOS 改用由 Tauri 層處理的 app-container 權利（entitlements）。
完整 iOS 支援延後至 v0.7.0+。

## 新增一個 target

1. 建立 `core/src/platform/<os>.rs`，內含 `Platform` struct、`PLATFORM`
   static 以及 `impl PlatformAdapter`。
2. 在 `mod.rs` 中加入一行 `#[cfg(target_os = "<os>")] mod <os>;`。
3. 為 `current()` 加入對應的 `#[cfg]` 分支。

略過步驟 2 或 3 會觸發 `compile_error!` 後援機制（fallback），因此建置不會
悄悄產出一個沒有轉接器的 OS。
