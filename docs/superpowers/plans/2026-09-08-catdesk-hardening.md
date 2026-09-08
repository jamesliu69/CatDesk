# CatDesk Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Harden CatDesk's filesystem boundary, Linux sandbox, browser bridge, daemon lifecycle, persistence, and runtime dependency behavior for reliable Raspberry Pi operation.

**Architecture:** Keep Cloudflare external and the MCP listener local. Strengthen boundaries inside CatDesk, add a headless supervised runtime path, and make state/dependency behavior deterministic without redesigning the MCP protocol.

**Tech Stack:** Rust 2024, Tokio, Axum, Bubblewrap/Landlock on Linux, systemd user services, Node/npm only for `chrome-devtools-mcp`.

**Spec:** `docs/superpowers/specs/2026-09-08-catdesk-hardening-design.md`

## Global Constraints

- Listener remains `127.0.0.1` only.
- Cloudflare Tunnel remains external to CatDesk.
- `upstream` is read-only; no upstream pushes or PRs.
- Each behavior fix follows RED → GREEN → regression verification.
- Final required gates: `cargo fmt --check`, `cargo test --release`, `cargo build --release`.

---

### Task 1: Symlink-safe workspace path resolution

**Files:**
- Modify: `src/command.rs`
- Test: `src/command.rs`

**Interfaces:**
- Produces: `resolve_workspace_path()` and `resolve_command_path()` that reject unresolved descendants of symlinks escaping the workspace.

- [ ] Add a Unix regression test that creates an external directory plus a workspace symlink to it and resolves `link/new-file.txt`.
- [ ] Run the targeted test and verify it fails on the current resolver.
- [ ] Add a helper that canonicalizes the nearest existing ancestor and appends only unresolved normal path components.
- [ ] Use the helper in both workspace path resolvers.
- [ ] Run command/workspace tests and `cargo test --release`.
- [ ] Commit as `fix: close workspace symlink escape`.

### Task 2: Bubblewrap Linux sandbox with fail-closed fallback

**Files:**
- Modify: `src/linux_sandbox.rs`
- Modify: `scripts/deploy-pi-local.sh`
- Modify: `CATDESK_PI_LANDLOCK_FIX.md`
- Test: `src/linux_sandbox.rs`, `src/process_runner.rs`

**Interfaces:**
- Produces: `helper_command()` selecting Bubblewrap when available, otherwise Landlock; no unsandboxed execution path.

- [ ] Add tests for Bubblewrap argument construction and for hiding an outside home path when `bwrap` is available.
- [ ] Verify the new tests fail before implementation.
- [ ] Build a Bubblewrap command with runtime paths read-only, workspace/scratch read-write, private `/tmp`, `/proc`, and `/dev`.
- [ ] Remove `CATDESK_ALLOW_UNSANDBOXED_LINUX` from runtime and Pi deployment.
- [ ] Run Linux sandbox/process-runner tests, then release tests/build.
- [ ] Commit as `fix: sandbox linux commands with bubblewrap`.

### Task 3: Race-safe concurrent DevTools bridge

**Files:**
- Modify: `src/devtools.rs`
- Modify call sites in `src/mcp.rs` and `src/main.rs` only as required by the bridge interface.
- Test: `src/devtools.rs`

**Interfaces:**
- Produces: browser JSON-RPC request registration before write and no bridge-wide mutex held while awaiting a response.

- [ ] Add deterministic tests around request registration/order using an in-memory/mock transport boundary extracted from the bridge.
- [ ] Verify tests fail with the current write-before-register behavior.
- [ ] Register pending request before write and remove pending entries on write failure/timeout.
- [ ] Separate stdin locking from response waiting; keep request IDs concurrency-safe.
- [ ] Surface child EOF to pending requests immediately.
- [ ] Run targeted tests and release tests/build.
- [ ] Commit as `fix: make devtools bridge race safe`.

### Task 4: Headless supervised service lifecycle

**Files:**
- Modify: `src/main.rs`
- Modify: `src/server.rs` / `src/state.rs` if lifecycle events require it.
- Modify: `scripts/deploy-pi-local.sh`
- Create/update Pi user service installation logic in the deploy script.
- Test: unit tests beside lifecycle parsing/state code.

**Interfaces:**
- Produces: `catdesk --headless` using persisted mode/public URL and a direct systemd service with `Restart=on-failure`.

- [ ] Add tests for CLI headless option parsing and service-exit state transition.
- [ ] Verify tests fail before implementation.
- [ ] Add headless event loop that drains UI events, starts services, flushes state on SIGINT/SIGTERM, and exits non-zero on fatal MCP server failure.
- [ ] Make the Axum task report exit/error instead of discarding it.
- [ ] Update Pi service setup to execute the real CatDesk binary directly; remove tmux ownership from service supervision.
- [ ] Run targeted tests and release tests/build.
- [ ] Commit as `feat: add supervised headless service mode`.

### Task 5: Atomic configuration persistence and bounded usage writes

**Files:**
- Modify: `src/state.rs`
- Test: `src/state.rs`

**Interfaces:**
- Produces: crash-safe `save_to_path()` and bounded token usage persistence.

- [ ] Add a test proving save replaces an existing file with valid complete TOML and leaves no temp file.
- [ ] Add a test for usage dirty/flush policy.
- [ ] Verify the behavior tests fail before implementation.
- [ ] Write config to a unique same-directory temp file, `sync_all`, rename atomically, and sync the parent directory on Unix.
- [ ] Mark usage dirty and flush at a bounded threshold; force flush during clean shutdown/settings persistence.
- [ ] Run state tests and release tests/build.
- [ ] Commit as `fix: make config persistence crash safe`.

### Task 6: Pin and diagnose chrome-devtools-mcp

**Files:**
- Modify: `src/devtools.rs`
- Modify: `README.md`, `README.zh-TW.md`, `docs/TECHNOLOGY.md`
- Test: `src/devtools.rs`

**Interfaces:**
- Produces: one explicit `chrome-devtools-mcp` version constant and bounded stderr diagnostics.

- [ ] Resolve the currently tested package version.
- [ ] Add a test asserting launch args use the pinned version rather than `@latest`.
- [ ] Implement the version constant and stderr capture/last-error reporting.
- [ ] Update documentation.
- [ ] Run targeted tests and release tests/build.
- [ ] Commit as `fix: pin chrome devtools mcp version`.

### Task 7: Public endpoint authentication decision and hardening

**Files:**
- Modify only if connector-compatible auth is verified: `src/server.rs`, `src/state.rs`, setup/docs.
- Otherwise modify documentation to specify Cloudflare Access as the supported outer authentication layer.

**Interfaces:**
- Maintains current connector operability while removing any misleading claim that the secret path is equivalent to user authentication.

- [ ] Verify current connector-supported authentication mechanism from authoritative documentation.
- [ ] If static Bearer/OAuth is directly compatible, add failing HTTP auth tests, implement auth, and update setup.
- [ ] If not compatible, keep secret-path routing and document/validate Cloudflare Access deployment instead of adding an unusable requirement.
- [ ] Run server tests and release tests/build.
- [ ] Commit as `docs:` or `feat:` according to the verified outcome.

### Task 8: Final quality gate and hardening review

**Files:**
- Modify only files required to fix regressions introduced by Tasks 1-7.

- [ ] Run `cargo fmt --check`.
- [ ] Run `cargo test --release` and confirm zero failures.
- [ ] Run `cargo build --release` and confirm exit code 0.
- [ ] Run `cargo clippy --all-targets` and record remaining pre-existing lint debt separately from hardening regressions.
- [ ] Inspect `git diff main...HEAD`, branch status, and commit list against this plan.
- [ ] Verify the running Pi service/tunnel after deployment changes without executing any physical-equipment action.
