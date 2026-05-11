# Mobile Web Mode (Tauri thin-shell)

Phantom Mesh ships a full React desktop UI (`app/src/`) and a separate
lightweight web frontend (`core/web/`, served by `phantom serve` on port
**7878**). On Android / iOS the Tauri shell skips the desktop UI and
loads the remote web frontend instead, so phones share the same surface
desktops see in a browser.

## How it works

`app/src/main.tsx` calls `maybeRedirectToRemoteFrontend()` from
`app/src/mobileThinShell.ts` *before* mounting React. The helper:

1. Detects a Tauri WebView running on Android/iOS.
2. On first launch (no host saved) renders a small in-page form asking
   for host + port.
3. On subsequent launches it `window.location.replace`s to
   `http://<host>:<port>/`, which is the URL `phantom serve` exposes.

Desktop builds short-circuit immediately (no Tauri-mobile UA → returns
`false`) and the React app mounts as before. No Rust, no Tauri config,
and no build-system changes were required.

## Workflow

1. On your Mac/PC, start the server:
   ```bash
   phantom serve            # listens on 0.0.0.0:7878
   ```
2. Connect your phone to the same Tailnet (recommended) or LAN.
3. Launch the Phantom Mesh Tauri app on the phone.
4. First launch: enter your Tailscale hostname (e.g. `mac-mini`) or LAN
   IP and tap **Connect**. Settings persist in `localStorage`.
5. Subsequent launches load the dashboard automatically.

## Resetting / changing the host

Open the app, then in the WebView console (or via desktop Safari /
Chrome remote-debug) run:

```js
localStorage.removeItem('PHANTOM_HOST');
location.reload();
```

Optional keys: `PHANTOM_PORT` (default `7878`), `PHANTOM_SCHEME`
(default `http`; set to `https` if you front the server with a TLS
proxy).
