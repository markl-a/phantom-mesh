# installers/

The exact PowerShell + shell scripts served by `https://phantommesh.io/install.ps1`
and `https://phantommesh.io/install.sh`. Read these BEFORE piping to
`iex` / `sh` from the network.

## What they do (both platforms)

1. **Stop any running `phantom`** so the binary file isn't locked.
2. **Download the platform-specific binary** from
   `https://phantommesh.io/dist/phantom-<platform>` to `~/.local/bin/phantom[.exe]`.
3. **Add `~/.local/bin` to your `PATH`** (User scope on Windows;
   `~/.bashrc` + `~/.zshrc` on Mac/Linux).
4. **Smoke test**: `phantom --version` to verify the binary runs.
5. **Seed `~/.phantom-mesh/agents.toml`** on FIRST install only. Default
   config has a 4-provider failover chain so a single LLM provider
   outage doesn't brick chat. Existing configs are left alone.
6. **Run `phantom login`** — opens browser → Google OAuth via
   phantommesh.io broker → vault auto-pulls your LLM API keys to
   `~/.phantom-mesh/env`. Skip via `PHANTOM_INSTALL_SKIP_LOGIN=1`.
7. **Auto-register this machine** with the broker's cluster peer
   registry, so other devices on the same Google account can dispatch
   work to it.
8. **Cluster sync** — pulls the latest peer list + cluster_secret into
   the local `[cluster]` block of `agents.toml`.
9. **Start `phantom serve`** in the background (Scheduled Task on
   elevated Windows / launchd plist on macOS / nohup on Linux).

## Where files land

| Path | What |
|---|---|
| `~/.local/bin/phantom[.exe]` | the CLI binary itself |
| `~/.phantom-mesh/agents.toml` | local config (providers, agents, cluster, workspace pin) |
| `~/.phantom-mesh/env` | LLM API keys synced from vault (sourced by serve at launch) |
| `~/.phantom-mesh/auth.json` | OAuth tokens — DO NOT share this file |
| `~/.phantom-mesh/peers.json` | broker-synced cluster peer list |

## What they DON'T do

- ✗ No `sudo` / admin needed (everything is user-scope)
- ✗ No telemetry beacon back to phantommesh.io beyond the OAuth flow
  + cluster heartbeat (which YOU explicitly opted into by running login)
- ✗ No background updates without `phantom cluster upgrade` from your side
- ✗ No data uploaded to phantommesh.io — only your LLM API keys are
  vaulted there (encrypted at rest with a per-user AES-256 key)

## Auditing tips

- The two `.ps1` / `.sh` files in this directory are EXACTLY what
  `https://phantommesh.io/install.{ps1,sh}` serves. Diff them:

  ```bash
  diff install.ps1 <(curl -s https://phantommesh.io/install.ps1)
  ```

  Mismatch = the broker is serving something different from this repo —
  please [report it](https://github.com/markl-a/phantom-mesh/issues/new).

- The downloaded binary at `~/.local/bin/phantom[.exe]` is the only
  closed-source piece. The Rust source for it is on track for May 2026
  open-source release per the main README.

## Safer one-liner (download first, inspect, then run)

```powershell
# Windows
iwr -useb https://phantommesh.io/install.ps1 -OutFile install.ps1
notepad install.ps1   # read it first
.\install.ps1         # then run if you trust it
```

```bash
# macOS / Linux
curl -fsSL https://phantommesh.io/install.sh -o install.sh
less install.sh       # read it first
sh install.sh         # then run if you trust it
```
