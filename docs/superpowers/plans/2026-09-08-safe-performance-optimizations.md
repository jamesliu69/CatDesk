# Safe Performance Optimizations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove repeated backend work while preserving complete terminal command change reports and current MCP behavior.

**Architecture:** Keep exact before/after snapshots for synchronous tools. For background jobs, defer the one complete after-snapshot until the job reaches a terminal state and cache that result in the job. Add metadata-keyed caches only around read-only config/instruction paths, and cache immutable widget resource encodings.

**Tech Stack:** Rust, Tokio, serde_json, existing `ignore`, `base64`, and `tiktoken-rs` dependencies.

**Spec:** `docs/superpowers/specs/2026-09-08-safe-performance-optimizations-design.md`

## Global Constraints

- Never use mtime or file size alone to produce the terminal `changedFiles` result.
- Keep `run_command`, `write`, `edit`, and `delete` exact before/after tracking unchanged.
- Do not change TUI redraw scheduling, token-count semantics, or remote-debug startup polling.
- Do not add a dependency for caching or file watching.
- Run `cargo fmt --check`, `cargo test --release`, and `cargo build --release` after implementation batches.

---

### Task 1: Defer and cache background-job change reports

**Files:**
- Modify: `src/command_jobs.rs:1-450`
- Test: `src/command_jobs.rs` unit-test module

**Interfaces:**
- Consumes: existing `CommandJob::runtime`, `CommandJob::change_session`, and `ChangeSession::changes()`.
- Produces: `CommandJobManager::current_changes()` that returns no intermediate changes while running and one cached complete result after terminal completion.

- [x] **Step 1: Add a regression test for terminal-only collection**

Create a temporary workspace, start a sleeping command with a recursive `ChangeSession`, create a file while the job is running, assert `current_changes()` is empty while running, wait for terminal state, then assert the terminal result includes the file and the second terminal query returns the same result.

```rust
#[tokio::test]
async fn background_change_report_is_deferred_and_cached_until_terminal() {
    let root = workspace("deferred-changes");
    let session = ChangeSession::begin(
        &root,
        ChangeScope::single(ChangeTarget::explicit(root.clone(), true)),
    );
    let command = if cfg!(windows) { "Start-Sleep -Milliseconds 300" } else { "sleep 0.3" };
    let manager = CommandJobManager::new();
    let started = manager
        .start_with_change_session(
            command.into(), root.clone(), root.clone(), 5_000, None, Some(session),
        )
        .await
        .expect("start job");

    let running = manager.current_changes(&started.snapshot.job_id).await.expect("running changes");
    assert!(running.is_empty());
    std::fs::write(root.join("created.txt"), "created\n").expect("create file");

    let terminal = wait_terminal(&manager, &started.snapshot.job_id).await;
    assert!(terminal.state.is_terminal());
    let first = manager.current_changes(&started.snapshot.job_id).await.expect("terminal changes");
    assert!(first.iter().any(|file| file.path == "created.txt"));
    let second = manager.current_changes(&started.snapshot.job_id).await.expect("cached changes");
    assert_eq!(first, second);
    let _ = std::fs::remove_dir_all(root);
}
```

- [x] **Step 2: Run the focused test and verify it fails**

Run: `cargo test background_change_report_is_deferred_and_cached_until_terminal -- --nocapture`

Expected: FAIL because running jobs currently scan and return the changed file, and terminal results are not cached.

- [x] **Step 3: Add one-time final-result storage**

Import `std::sync::OnceLock`, add `final_changes: OnceLock<Vec<FileChange>>` to `CommandJob`, initialize it in `new_with_change_session`, and update `current_changes`:

```rust
let snapshot = job.snapshot(0).await;
if !snapshot.state.is_terminal() {
    return Ok(Vec::new());
}
let Some(session) = job.change_session.as_ref() else {
    return Ok(Vec::new());
};
Ok(job
    .final_changes
    .get_or_init(|| session.changes())
    .clone())
```

The final closure must call the existing full `ChangeSession::changes()` implementation; do not add metadata-only logic to this path.

- [x] **Step 4: Run the focused test and the command-job tests**

Run: `cargo test command_jobs::tests -- --nocapture`

Expected: PASS, including the new deferred/cached report test.

- [x] **Step 5: Commit the isolated change-tracking behavior**

```bash
git add src/command_jobs.rs
git commit -m "perf: defer background change scans until completion"
```

### Task 2: Cache config and instruction reads without stale errors

**Files:**
- Modify: `src/mcp.rs:1-21, 1950-2160, 2670-2675`
- Test: `src/mcp.rs` unit-test module

**Interfaces:**
- Consumes: existing `app_config_path()`, `load_app_config()`, `agents_widget_state()`, and `read_agents_text()`.
- Produces: private metadata-keyed cached loaders and a text-based structured instruction helper.

- [x] **Step 1: Add cache invalidation tests**

Add a direct helper test that writes `AGENTS.md`, calls the new cached text loader twice, updates the file, and asserts the returned value changes. Use a unique temporary directory and avoid asserting implementation details such as cache hit counters.

```rust
#[test]
fn cached_instruction_text_reloads_after_agents_file_changes() {
    let root = std::env::temp_dir().join(format!("catdesk-instruction-cache-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("create workspace");
    let agents = root.join("AGENTS.md");
    std::fs::write(&agents, "first instruction\n").expect("write first instructions");
    assert_eq!(cached_agents_text(&agents).as_deref(), Some("first instruction"));
    std::fs::write(&agents, "second instruction\n").expect("write second instructions");
    assert_eq!(cached_agents_text(&agents).as_deref(), Some("second instruction"));
    let _ = std::fs::remove_dir_all(root);
}
```

- [x] **Step 2: Run the focused cache test and verify it fails**

Run: `cargo test cached_instruction_text_reloads_after_agents_file_changes -- --nocapture`

Expected: FAIL to compile because `cached_agents_text` does not exist yet.

- [x] **Step 3: Implement metadata-keyed cached loaders**

Add a small private cache record in `mcp.rs` keyed by absolute path, file length, and `metadata.modified()`. Cache only successful reads/parses; on metadata or read/parse errors, bypass/update no cache and return the original error/`None` behavior. Use `Mutex` plus `OnceLock` from the standard library. Replace the MCP call sites in `agents_widget_state()` and `current_token_stats_layout()` with the cached config loader, and use the cached text loader from `preferred_agents_text()`.

- [x] **Step 4: Reuse instruction text for the plain and structured response**

Keep the existing `catdesk_instruction_structured()` helper for tests and compatibility, add `catdesk_instruction_structured_from_text(&str) -> Value`, and have `handle_catdesk_instruction_with_show_detail_mode()` pass the already computed `instruction_text` to the new helper instead of calling `catdesk_instruction_text()` a second time.

- [x] **Step 5: Run MCP instruction/state tests**

Run: `cargo test mcp::tests -- --nocapture` and `cargo test state::tests -- --nocapture`

Expected: PASS with unchanged instruction contents and widget fields.

- [x] **Step 6: Commit config/instruction caching**

```bash
git add src/mcp.rs
git commit -m "perf: cache repeated MCP instruction reads"
```

### Task 3: Reuse payload conversion and static PNG encodings

**Files:**
- Modify: `src/mcp.rs:364-390, 2540-2690`
- Test: `src/mcp.rs` unit-test module

**Interfaces:**
- Consumes: existing `FileChange`, `AutoWidgetContext`, `file_entry_json()`, and compile-time PNG byte arrays.
- Produces: unchanged structured/widget payloads backed by shared conversion helpers and one-time PNG base64 strings.

- [x] **Step 1: Add payload equivalence tests**

Extend `changed_files_reach_the_model_not_only_the_widget` with an `AutoWidgetContext` using the same `FileChange`, then assert the widget entries equal the structured entries. Add a resource assertion that renders `/ui://widget/catdesk-dashboard.html` and checks all three embedded image placeholders were replaced.

```rust
let context = AutoWidgetContext {
    is_error: false,
    turn_files: vec![change("a.rs", "@@ -1 +1 @@\n-a\n+b\n")],
};
let (widget_files, has_changes) = widget_changed_files(Some(&context));
assert!(has_changes);
assert_eq!(widget_files, result["structuredContent"]["changedFiles"]);
let html = render_widget_html("ui://widget/catdesk-dashboard.html", 1);
assert!(!html.contains(REENABLE_WIDGET_IMAGE_PLACEHOLDER));
assert!(!html.contains(REFRESH_CATDESK_IMAGE_PLACEHOLDER));
assert!(!html.contains(REMOVE_CATDESK_IMAGE_PLACEHOLDER));
```

- [x] **Step 2: Extract one shared changed-file value list**

Add `changed_files_json: Vec<Value>` to `AutoWidgetContext`, initialize it once from `turn_files` in `handle_tools_call_with_show_detail_mode`, and have both `attach_changed_files` and `widget_changed_files` clone from that list. Apply the existing 4,000-byte model diff cap to the structured clone only; preserve `changedFileDiffsOmitted` and `changedFilesAtCap` exactly.

- [x] **Step 3: Cache static PNG base64 strings with `OnceLock`**

Add one `OnceLock<String>` per embedded PNG and replace the three per-call `.encode(...)` expressions in `render_widget_html()` with the cached strings. Keep the `data:image/png;base64,` prefix unchanged.

- [x] **Step 4: Run focused MCP tests and commit**

Run: `cargo test mcp::tests -- --nocapture`

```bash
git add src/mcp.rs
git commit -m "perf: reuse widget payload and resource encodings"
```

### Task 4: Cache search backend availability probes

**Files:**
- Modify: `src/workspace_tools.rs:731-776`
- Test: `src/workspace_tools.rs` unit-test module

**Interfaces:**
- Consumes: existing `search_with_backend()`, `search_text_rg()`, and `search_text_grep()` behavior.
- Produces: process-wide cached availability decisions with the same backend error/fallback semantics.

- [x] **Step 1: Add a backend-selection behavior test**

Keep the existing `search-rg-dialect`, `grep_search_backend_marks_truncated_when_an_extra_match_exists`, and Rust-backend tests as the observable contract; no new host-dependent test is required because the cache only changes probe frequency, not selected backend output.

- [x] **Step 2: Cache availability probes**

Use `OnceLock<bool>` for the `rg` and `grep` availability checks. If the selected executable later disappears, preserve the current `SearchBackendError::Unavailable` handling and do not silently return incomplete results.

- [x] **Step 3: Run workspace search tests and commit**

Run: `cargo test workspace_tools::tests -- --nocapture`

```bash
git add src/workspace_tools.rs
git commit -m "perf: cache search backend probes"
```

### Task 5: Full verification and handoff

**Files:**
- Modify: none unless formatting requires it

- [x] **Step 1: Run formatting and all release tests**

Run: `cargo fmt --check` then `cargo test --release`.

Expected: all tests pass with zero failures.

- [x] **Step 2: Build the optimized binary**

Run: `cargo build --release`.

Expected: optimized CatDesk binary builds successfully.

- [x] **Step 3: Review the diff and working tree**

Run: `git diff main...HEAD --stat`, `git diff main...HEAD --check`, and `git status --short`.

Expected: only the approved spec, plan, and performance implementation files are changed; no generated files or secrets are present.
