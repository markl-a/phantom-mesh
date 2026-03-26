---
name: Bug Report
about: Report a bug to help us improve Phantom Mesh
title: "[Bug] "
labels: bug
assignees: ''
---

## Description

A clear and concise description of the bug.

## Steps to Reproduce

1. Start the daemon with `cargo run -- daemon`
2. Send the following command / API request: ...
3. Observe the error

## Expected Behavior

What you expected to happen.

## Actual Behavior

What actually happened. Include error messages, logs, or stack traces if available.

## Environment

- **OS**: (e.g., Windows 11, macOS 14, Ubuntu 24.04)
- **Rust version**: (output of `rustc --version`)
- **Phantom Mesh version/commit**: (output of `git rev-parse --short HEAD`)
- **Provider(s) in use**: (e.g., ollama, gemini, groq)
- **Relevant config**: (sanitized excerpt from `~/.phantom-mesh/agents.toml`, if applicable)

## Logs

<details>
<summary>Daemon logs</summary>

```
Paste relevant log output here (use RUST_LOG=debug for verbose output).
Remove any API keys or secrets before posting.
```

</details>

## Additional Context

Any other context about the problem (screenshots, related issues, workarounds tried).
