# CatDesk Upstream v0.9.1 Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Integrate the selected upstream v0.9.1 fixes and UI improvements while preserving the fork's Cloudflare/external tunnel, Landlock fallback, and hardening/performance changes.

**Architecture:** Perform feature-level ports from upstream commits instead of a wholesale merge because the fork's v0.8 integration is not in the same Git ancestry. Preserve fork-specific interfaces (`public_base_url`, Landlock fallback) and selectively transplant upstream behavior with regression tests before each implementation step.

**Tech Stack:** Rust 2024, Tokio, Axum, Ratatui, Bubblewrap/Landlock, HTML/CSS widget, Cargo.

**Spec:** `docs/superpowers/specs/2026-09-17-upstream-v091-integration-design.md`

## Global Constraints

- `upstream` is fetch-only; never push to it.
- Do not restore ngrok or `src/ngrok.rs`.
- Preserve `public_base_url` external HTTPS tunnel support.
- Preserve Bubblewrap + Landlock fail-closed fallback behavior.
- Preserve fork-specific performance/startup/DevTools/workspace hardening.
- Final verification: `cargo fmt --check`, `cargo test --release`, `cargo build --release`.

---

### Task 1: Sandbox process lifetime and Git worktree metadata

**Files:**
- Modify: `src/process_runner.rs`
- Modify: `src/linux_sandbox.rs`
- Modify: `Cargo.toml` only if required by retained Landlock behavior

**Interfaces:**
- Consumes: existing `linux_sandbox::helper_command(command, workspace_root, cwd)` and `PreparedShellCommand` flow.
- Produces: long-running Bubblewrap child processes that survive Tokio blocking-worker retirement and `workspace_git_paths` validation equivalent to upstream v0.9.1 while preserving Landlock fallback.

- [ ] **Step 1: Add the upstream process-lifetime regression test before production changes**

Use the behavior from upstream commit `dac5803`: create a Bubblewrap-backed child from a blocking worker, retire/drop the creating worker, and assert the command still completes when spawned through the corrected path.

- [ ] **Step 2: Run the focused test and confirm the current implementation fails on Linux**

```powershell
cargo test --release process_runner::tests:: -- --nocapture
```

On Windows, compile the test-gated code and use the existing unit suite as the available platform check; Linux-specific execution is validated by the test definition and final CI-compatible build.

- [ ] **Step 3: Port the `dac5803` preparation/spawn split**

Keep preparation inside `spawn_blocking`, but call the final spawn helper on the Tokio runtime worker:

```rust
let prepared = tokio::task::spawn_blocking(move || {
    shell_command(&command, &workspace_root, &cwd_for_prepare)
})
.await
.map_err(|error| io::Error::other(format!("command preparation task failed: {error}")))??;

spawn_prepared_shell_command(prepared, cwd)
```

- [ ] **Step 4: Port upstream v0.9.1 worktree/external Git metadata validation**

Use the behavior from commits `f17096f`, `b39e4d0`, `53b2273`, and `0415008`: accept valid sibling/nested linked worktrees and external Git object metadata, reject fake/symlink-escaped metadata, and only bind validated paths. Keep the existing Landlock path as the fallback when no trusted `bwrap` exists.

- [ ] **Step 5: Run focused sandbox/process tests**

```powershell
cargo test --release linux_sandbox::tests:: -- --nocapture
cargo test --release process_runner::tests:: -- --nocapture
```

### Task 2: Local-time logging

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/state.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Produces: `local_now() -> OffsetDateTime` and local timestamps for user-facing/runtime logs.

- [ ] **Step 1: Add/port the local-time tests from upstream commit `c227e68`**

The tests must verify that formatted current timestamps use the local UTC offset rather than always using UTC.

- [ ] **Step 2: Run the focused tests before implementation**

```powershell
cargo test --release state::tests:: -- --nocapture
```

- [ ] **Step 3: Port `local_now` and replace UTC-only log timestamp call sites**

Use the `time` crate's local-offset support exactly where upstream v0.9.1 does, without adding ngrok state.

- [ ] **Step 4: Run state/main tests**

```powershell
cargo test --release state::tests:: -- --nocapture
cargo test --release main::tests:: -- --nocapture
```

### Task 3: ChatGPT-aligned widget and settings reachability

**Files:**
- Modify: `src/widget/catdesk_dashboard.html`
- Modify: `src/mcp.rs`
- Modify: `src/state.rs`
- Modify: `src/main.rs`
- Modify: `src/server.rs` only where the widget metadata interface requires it

**Interfaces:**
- Consumes: existing `public_base_url`, `mcp_slug`, co-author setting, widget resource generation.
- Produces: upstream-equivalent theme-safe ChatGPT surfaces and `WidgetCornerStyle` selection while keeping external tunnel configuration.

- [ ] **Step 1: Port/add tests for widget corner style and theme-safe resource output**

Use behavior from upstream commits `9192dce`, `b64c0c2`, `10114e9`, `6864e16`, `2f88fb3`, and `db851c0`.

- [ ] **Step 2: Run widget/MCP/state focused tests before implementation**

```powershell
cargo test --release mcp::tests:: -- --nocapture
cargo test --release state::tests:: -- --nocapture
```

- [ ] **Step 3: Port the widget HTML/CSS changes and `WidgetCornerStyle` state**

Do not import upstream ngrok runtime fields. Keep widget CSP/public-action origin generation based on `public_base_url`.

- [ ] **Step 4: Adapt the upstream settings reachability fix**

Use commit `9ff7b9d` as the navigation/layout reference. Settings rows remain: theme/tool/detail/corner style, co-author, MCP slug actions, and `public_base_url` edit action. No ngrok rows are added.

- [ ] **Step 5: Run focused UI/state/MCP tests**

```powershell
cargo test --release mcp::tests:: -- --nocapture
cargo test --release state::tests:: -- --nocapture
cargo test --release main::tests:: -- --nocapture
```

### Task 4: Version metadata and full validation

**Files:**
- Modify: `Cargo.toml`
- Modify: `package.json`
- Regenerate: `Cargo.lock`
- Update documentation only where version/tunnel wording becomes inaccurate

**Interfaces:**
- Produces: fork metadata based on upstream v0.9.1 with no ngrok regression.

- [ ] **Step 1: Set both package versions to `0.9.1`**

```toml
version = "0.9.1"
```

```json
"version": "0.9.1"
```

- [ ] **Step 2: Regenerate dependency lock metadata**

```powershell
cargo check --release
```

- [ ] **Step 3: Check for accidental ngrok restoration and diff hygiene**

```powershell
git grep -n "ngrok" -- Cargo.toml src README.md README.zh-TW.md
git diff --check
```

Expected: no runtime ngrok dependency/module/state is restored; historical/comparison documentation may mention ngrok only when clearly intentional.

- [ ] **Step 4: Run complete validation**

```powershell
cargo fmt --check
cargo test --release
cargo build --release
```

- [ ] **Step 5: Review final diff against the spec**

Confirm Cloudflare/external `public_base_url`, Landlock fallback, performance caches, non-blocking startup, DevTools hardening, and workspace hardening remain present.
