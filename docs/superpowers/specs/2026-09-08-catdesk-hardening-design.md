# CatDesk Hardening Design

## Goal

Harden CatDesk for long-running Raspberry Pi use without changing the Cloudflare Tunnel topology that is already working. Fix workspace boundary escapes, remove unsandboxed Linux command execution on the Pi, make browser requests race-safe, make the server process supervisable, make state persistence crash-safe, and remove avoidable runtime version drift.

## Constraints

- Keep the MCP listener bound to `127.0.0.1`.
- Keep Cloudflare Tunnel external to CatDesk.
- Preserve the current MCP tool surface unless a security boundary requires stricter behavior.
- Do not write to `upstream`; all branch work stays local until explicitly pushed to `origin`.
- Follow TDD for behavior changes and run `cargo fmt --check`, `cargo test --release`, and `cargo build --release` before completion.

## Design

### 1. Workspace path boundary

Replace the current `canonicalize().unwrap_or(candidate)` behavior with a resolver that canonicalizes the nearest existing ancestor and then appends unresolved path components. This makes a path such as `workspace/link/new.txt`, where `link` points outside the workspace, resolve against the real external parent and fail the workspace-prefix check. Existing paths continue to use normal canonicalization.

### 2. Linux command sandbox

Linux command execution becomes fail-closed. When `bwrap` is available, CatDesk uses Bubblewrap to construct a filesystem namespace containing system runtime paths read-only, the workspace read-write, and a private scratch directory. Sensitive home-directory content is absent unless it is one of the narrow runtime paths already allowed by the Landlock policy. If Bubblewrap is unavailable, CatDesk uses the existing Landlock helper. The `CATDESK_ALLOW_UNSANDBOXED_LINUX` escape hatch is removed from deployment and runtime behavior.

### 3. DevTools bridge

Register the JSON-RPC request ID in `pending` before writing the request to the child process. Remove timed-out/failed pending entries. Separate stdin serialization from response waiting so one slow browser request does not hold an outer bridge lock for up to 120 seconds. Child EOF is treated as bridge failure rather than silently leaving callers to time out.

### 4. Service lifecycle

Add a headless service mode that runs the existing MCP server without requiring a tmux-owned TUI session. The Raspberry Pi user service will execute CatDesk directly with `Restart=on-failure`, allowing systemd to supervise the real process. Server task exit updates application state and is surfaced to the headless loop as a fatal service failure so the process exits non-zero and systemd can restart it.

### 5. Persistence

Write `config.toml` through a unique temporary file in the same directory, `sync_all`, and atomic `rename`; sync the directory on Unix. This prevents power loss during a truncate/write cycle from leaving a partially written configuration. Token usage persistence will be reduced from every tool call to a bounded periodic/threshold write while clean shutdown still flushes state.

### 6. Browser dependency reproducibility

Replace `chrome-devtools-mcp@latest` with one tested explicit version. Capture a bounded amount of stderr so startup/runtime failures are diagnosable.

### 7. Public endpoint hardening

Keep the high-entropy secret path because it is compatible with the current connector. Add optional Bearer authentication only if the current ChatGPT connector can supply it without breaking the installed integration; otherwise document Cloudflare Access as the authentication layer rather than shipping an unusable server-side requirement.

### 8. Quality gates

Add regression tests for each functional fix. Preserve the repository-required release test/build gates. Address Clippy findings touched by this work and establish a practical Clippy gate only after existing unrelated lint debt is separated from functional hardening.
