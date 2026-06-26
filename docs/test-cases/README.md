# Test Case DB Shared Schema

> Shared dictionary for the seven surface case DBs: `mac.md`, `win-cli.md`,
> `linux-cli.md`, `mac-app.md`, `win-app.md`, `android-app.md`, and `ios-app.md`.
> Surface files may add explicit extensions, but these base meanings are stable.

## Row Schema

| Column | Meaning |
|---|---|
| `ID` | Stable case id; never reused after deletion or retirement. |
| `Type` | `unit`, `integ`, `e2e`, `manual`, `static`, or `monitor`. Legacy `grep` means `static`. |
| `Auto` | Automation level: `✅` fully automatic, `⚠` needs env/fixture/device, `❌` manual, `⏰` scheduled/cron, `🔒` blocked by known code-backlog. |
| `Setup` | Required preconditions before the command or action. |
| `cmd` | Command, script, static assertion, or manual action the runner/operator performs. |
| `expected` | Passing condition. For `🟥` rows this is the target assertion, not today's result. |
| `Verifies` | Flow/spec/charter invariant covered by the case. |
| `last_run` | Last verified date plus runner, or `⬜` when not yet run. |
| `狀態` | Current verdict token, using the status dictionary below. |

## Status Tokens

| Token | Meaning |
|---|---|
| `✅` | Passing / proven existing behavior. |
| `🟡` | Partial, degraded, debt, or needs env/manual verification. |
| `🔴` | Important missing capability or release-risk hole; not the same as a known failing target assertion. |
| `⬜` | Not run / not yet verified. |
| `🟥` | FAIL / code-backlog: the assertion is expected to fail today due to a known code gap. Runners should log it as known-FAIL, not block unrelated green cases, and Charter completion must reconcile it. |
| `⏸` | Deferred out of the current ship gate, usually v0.7.0+. |
| `❔` | Unknown / needs triage. |
| `♻️` | Retired or drift-only historical case; not a live target assertion. |

Surface files should link here from §0 instead of redefining conflicting meanings.
