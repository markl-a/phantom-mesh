# 在 Oracle Cloud（Linux VM）上安裝 Phantom Mesh

**目標讀者：** 要在 Oracle Cloud Infrastructure（OCI，甲骨文雲端基礎設施）Always-Free（永久免費）方案上部署 phantom-mesh coordinator（協調者）節點的 operator（操作者）。

**標準目標**（依 `docs/SPEC-FREEZE-V1.md` §6.1）：**A1 ARM**
（`VM.Standard.A1.Flex`，最多 4 OCPU / 24 GB RAM，`aarch64-unknown-linux-gnu`）。

**鷹架目標**（Path C，在搶到 A1 容量之前）：**AMD micro**
（`VM.Standard.E2.1.Micro`，1 OCPU / 1 GB RAM，`x86_64-unknown-linux-gnu`）。

同一套流程在兩者上都適用；唯一的差異會就地標註。

---

## 1. OCI 主控台 — 一次性設定

這些步驟必須在 Oracle Cloud 網頁主控台中完成。setup（安裝設定）腳本無法觸及 OCI APIs。

### 1.1 佈建 VM

| 設定 | 值 |
|---|---|
| Image（映像檔） | Oracle Linux 9（RHEL 系列）**或** Ubuntu 22.04 LTS |
| Shape（機型） | `VM.Standard.A1.Flex`（4 OCPU / 24 GB）— 首選，"Always Free" |
| ↳ 後備選項 | `VM.Standard.E2.1.Micro`（1 OCPU / 1 GB）— 當 A1 出現 "Out of host capacity"（主機容量不足）時 |
| Region（區域） | 離 operator 最近者（Singapore、Tokyo、Osaka、Seoul） |
| SSH key | 貼上你的公鑰 |
| VCN | 啟動時建立的預設值即可 |

熱門亞洲區域的 A1 容量經常滿載 — 請每隔幾小時持續重試啟動，或寫一個輪詢用的 cron（定時排程）。

### 1.2 指派公開 IP

主控台 → Compute → Instances → *你的 VM* → Attached VNICs → Primary VNIC →
**Reserve a public IP**（保留一個公開 IP）（或指派一個臨時的）。

### 1.3 Security List（安全清單）入站規則

主控台 → Networking → Virtual Cloud Networks → *你的 VCN* → Security Lists →
*Default Security List* → Add Ingress Rules（新增入站規則）：

| Source CIDR（來源 CIDR 網段） | Protocol（協定） | Dest Port（目標埠） | Notes（備註） |
|---|---|---|---|
| `<your home IP>/32` | TCP | `22` | SSH 引導（替換成你實際的 IP） |
| `0.0.0.0/0` | UDP | `41641` | Tailscale STUN — 直連路徑所需 |

**不要**把 `7878/tcp` 加進公開清單。phantom daemon（常駐服務）只透過 Tailscale 連接；腳本的 `firewalld`/`ufw` 規則已經把它限制在 `tailscale0` 介面上。

### 1.4 SSH 引導

```bash
# ~/.ssh/config on operator's laptop
Host oci-coord
  HostName <PUBLIC_IP>
  User opc                  # opc on Oracle Linux; ubuntu on Ubuntu
  IdentityFile ~/.ssh/id_ed25519
```

測試：`ssh oci-coord true` — 必須成功才能繼續往下。

---

## 2. Tailscale 預先授權金鑰（pre-auth key）

phantom-mesh 假設這台 VM 會加入一個 Tailscale tailnet（私有網路）（依
`docs/MULTI-DEVICE-COORDINATION.md` Rule 4 + `docs/SESSION-ONBOARDING.md`
§3.2）。setup 腳本需要以非互動方式取得這把金鑰：

1. 前往 <https://login.tailscale.com/admin/settings/keys>
2. **Generate auth key**（產生授權金鑰）→ reusable=false、ephemeral=false、tag=`tag:phantom-mesh`
3. 複製產生出來的 `tskey-auth-...`（只顯示一次）

你會把它當作 `TAILSCALE_AUTH_KEY` 傳給 `setup-oci.sh`。

---

## 3. 在 VM 上 — clone、build、設定

### 3.0 把原始碼放到 VM 上

這個 repo（程式碼倉庫）是私有的，所以單純的 `git clone https://...` 會卡在等待認證憑證。三選一：

**選項 A — `gh` CLI（建議；設定最少；支援 `git pull`）：**

```bash
ssh oci-coord
sudo dnf install -y gh git                  # Oracle Linux 9
# OR: sudo apt-get install -y gh git        # Ubuntu / Debian
gh auth login                                # GitHub.com → HTTPS → device code
gh repo clone markl-a/phantom-mesh ~/repos/phantom-mesh
cd ~/repos/phantom-mesh && git checkout platform/linux
```

**選項 B — SSH deploy key（部署金鑰）（最安全；可撤銷；`git pull` 可用）：**

```bash
ssh oci-coord
ssh-keygen -t ed25519 -f ~/.ssh/gh-phantom -C "oci-coord" -N ""
cat ~/.ssh/gh-phantom.pub
# Copy the printed line. In a browser:
#   github.com/markl-a/phantom-mesh/settings/keys → Add deploy key
#   Title: oci-coord    Allow write access: NO    Key: paste pubkey

cat >> ~/.ssh/config <<'EOF'
Host github-phantom
  HostName github.com
  User git
  IdentityFile ~/.ssh/gh-phantom
EOF
chmod 600 ~/.ssh/config ~/.ssh/gh-phantom

git clone git@github-phantom:markl-a/phantom-mesh.git ~/repos/phantom-mesh
cd ~/repos/phantom-mesh && git checkout platform/linux
```

**選項 C — 從 operator 的機器 `scp` 一個 tarball（壓縮封存檔）過去（VM 上不需要 GitHub 認證）：**

從 operator 的本地 repo（Windows PowerShell 或 macOS/Linux shell）：

```bash
cd <repo-root>
tar --exclude='target' --exclude='node_modules' --exclude='dist' \
    --exclude='.worktrees' -czf /tmp/phantom-mesh.tar.gz .
scp /tmp/phantom-mesh.tar.gz oci-coord:~/
ssh oci-coord '
  mkdir -p ~/repos/phantom-mesh &&
  tar -xzf ~/phantom-mesh.tar.gz -C ~/repos/phantom-mesh &&
  cd ~/repos/phantom-mesh && git checkout platform/linux
'
```

Tarball 約 200–400 MB；在 E2.1.Micro 的 0.5 Gbps 上行頻寬下，預期約 1–2 分鐘。
首次部署後的更新仍需要在 VM 上具備 GitHub 存取權（或重新 scp），所以這只是一次性的引導選項。

### 3.1 建置二進位檔（binary）

```bash
# Install Rust if needed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env

# Distro deps for build
sudo dnf install -y gcc gcc-c++ openssl-devel pkgconf-pkg-config       # Oracle Linux / RHEL
# OR
sudo apt-get update && sudo apt-get install -y build-essential pkg-config libssl-dev   # Ubuntu / Debian

# Native release build
./scripts/build-linux.sh
```

這會把二進位檔放到 `dist/phantom-<host-triple>`。

**在 E2.1.Micro（1 GB RAM）上**，建置會在連結（link）階段因 OOM（記憶體耗盡）被強制終止。有兩個選項：

- **先加上 swap（置換空間）**（build-linux 腳本會警告；setup-oci 稍後會做，但你在建置前就需要它）：
  ```bash
  sudo fallocate -l 2G /swapfile && sudo chmod 600 /swapfile
  sudo mkswap /swapfile && sudo swapon /swapfile
  echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab
  ```
  建置會成功，但約需 15–25 分鐘。

- **或在別處建置後 scp**：在效能較強的 Linux host（主機）上交叉編譯（cross-compile），然後
  `scp dist/phantom-x86_64-unknown-linux-gnu oci-coord:~/repos/phantom-mesh/dist/`。
  這不被 `build-linux.sh` 支援（依 spec §6.1，偏好在目標機上建置）。

### 3.2 執行 setup-oci.sh

```bash
TAILSCALE_AUTH_KEY=tskey-auth-XXXXXXXX \
NODE_NAME=oci-singapore-coord \
  ./scripts/setup-oci.sh
```

這是冪等的（idempotent，可重複執行而結果相同）— 隨時可重跑。它會執行：

1. Swap 檔（若 RAM <2 GB）
2. 套件相依（package deps）
3. Tailscale 安裝 + 非互動 `up`
4. 防火牆：7878/tcp 僅限 `tailscale0`
5. `~/.local/bin/phantom` 安裝 + SELinux context（安全脈絡）（RHEL）
6. systemd **user**（使用者層級）unit（`~/.config/systemd/user/phantom-mesh.service`）
7. `loginctl enable-linger`（讓服務跨登出仍持續運作）
8. 從 `configs/agents.cloud.toml` 產生 `~/.phantom-mesh/agents.toml`
9. `~/.phantom-mesh/local.toml` 骨架 + `~/.phantom-mesh/env` 骨架

**它不會啟動 daemon** — 因為機密（secrets）還沒填入。

### 3.3 填入機密（secrets）

```bash
# 1. Cluster secret (must match every other node)
$EDITOR ~/.phantom-mesh/local.toml
# Paste cluster_secret = "<32-byte hex from 1Password / Mac>"

# 2. API keys
$EDITOR ~/.phantom-mesh/env
# Uncomment + fill:
#   ANTHROPIC_API_KEY=sk-ant-...
#   OPENROUTER_API_KEY=sk-or-...
#   TELEGRAM_BOT_TOKEN=...      (optional, only if this node is the bot gateway)
chmod 600 ~/.phantom-mesh/env ~/.phantom-mesh/local.toml
```

### 3.4 啟動 + 驗證

```bash
systemctl --user start phantom-mesh
journalctl --user -fu phantom-mesh        # tail logs

# In another shell:
curl -s http://localhost:7878/rpc/ping
# Expected JSON: {"core_sha": "...", "wire_version": 1, "phantom_version": "0.4.0", ...}

# From another tailnet device (Mac, phone, etc):
curl -s http://oci-singapore-coord:7878/rpc/ping
# Same response — confirms Tailscale + firewall rules let tailnet through

# From outside the tailnet (e.g. laptop without Tailscale, using public IP):
curl -m 5 http://<PUBLIC_IP>:7878/rpc/ping
# MUST fail (timeout / refused). If it succeeds, firewall is wrong.
```

---

## 4. 每日驗證（依 Rule 6）

```bash
phantom doctor --mesh
```

依 `docs/MULTI-DEVICE-COORDINATION.md` Rule 6 有三種 exit code（結束代碼）：
- `0` 所有 peers（對等節點）皆綠燈
- `1` 降級（degraded，有部分警告）
- `2` 損壞（broken，HMAC 不符 / schema 不符）

若 `phantom doctor --mesh` 標示這個節點與 Mac reference impl（參考實作）之間有 `core_sha` 漂移（drift），請執行 `phantom upgrade`（或透過 `git
pull && ./scripts/build-linux.sh && systemctl --user restart phantom-mesh` 重新建置）。

---

## 5. 疑難排解

### `MemoryMax` OOM（E2.1.Micro）

systemd 使用者 unit 預設不會釘住（pin）記憶體上限。若 daemon 在 1 GB 機器上於負載中被 OOM 強制終止：

```ini
# Add to ~/.config/systemd/user/phantom-mesh.service [Service] section:
MemoryMax=600M
MemoryHigh=500M
```

接著執行 `systemctl --user daemon-reload && systemctl --user restart phantom-mesh`。

### SELinux 拒絕（denials）（Oracle Linux）

```bash
sudo ausearch -m avc -ts recent | grep phantom
sudo audit2why -al | grep phantom
```

若 `bin_t` context 未被套用：
```bash
sudo semanage fcontext -a -t bin_t "$HOME/.local/bin/phantom"
sudo restorecon -v "$HOME/.local/bin/phantom"
```

最後手段（別讓它一直開著）：`sudo setenforce 0`。

### firewalld zone（區域）診斷（RHEL）

```bash
sudo firewall-cmd --list-all-zones | grep -A8 trusted
# Expect: interfaces: tailscale0  /  ports: 7878/tcp
```

### `loginctl enable-linger` 失敗

沒有 linger（停留），使用者層級的 systemd 服務會在 session（工作階段）結束時停止。
請驗證：
```bash
loginctl show-user "$(whoami)" | grep -i linger
# Linger=yes
```

### Tailscale 授權金鑰過期 / 無效

在 <https://login.tailscale.com/admin/settings/keys> 重新產生，然後：
```bash
TAILSCALE_AUTH_KEY=tskey-auth-... ./scripts/setup-oci.sh
```
（冪等 — 只會重跑缺漏的步驟）。

### 用 1 GB RAM 建置即使加了 swap 仍持續 OOM

```bash
# Reduce parallelism so peak memory is lower:
CARGO_BUILD_JOBS=1 ./scripts/build-linux.sh
```

---

## 6. 從 E2.1.Micro 遷移到 A1（當容量釋出時）

Path C 交接：

1. 依 §1.1 佈建新的 A1 instance
2. SSH 到 A1，執行 `git clone ...`、`git checkout platform/linux`
3. 執行 `./scripts/build-linux.sh` → 產生標準的 `dist/phantom-aarch64-unknown-linux-gnu`
4. 從舊節點的 `~/.phantom-mesh/local.toml` 複製 `cluster_secret`
5. 在 A1 上以相同的 `NODE_NAME` 執行 `setup-oci.sh`
6. 驗證 mesh（網狀網路）能連到它（在 Mac 上執行 `phantom doctor --mesh`）
7. 停止舊的 E2.1.Micro 服務：`systemctl --user stop phantom-mesh`
8. 經過一段觀察期（soak period）後，可選擇終止該 E2.1.Micro VM

A1 二進位檔是 spec 標準的成品（artefact，依 SPEC-FREEZE-V1 §6.1）。
E2.1.Micro 二進位檔只是鷹架 — 並非發行目標。

---

## 7. 交叉引用

- `docs/SESSION-ONBOARDING.md` §3.2 — 當你開啟 Oracle Cloud session 進行開發時的任務清單
- `docs/MULTI-DEVICE-COORDINATION.md` §Rule 4 — 設定拆分的理由
- `docs/SPEC-FREEZE-V1.md` §6.1 — 標準成品對照表
- `templates/phantom-mesh.service.tmpl` — 此腳本渲染（render）所用的 systemd 範本
- `scripts/build-linux.sh` — 產生二進位檔
- `scripts/setup-oci.sh` — VM 端建置後部署
