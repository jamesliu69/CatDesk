# Safe Performance Optimizations Design

**Date:** 2026-09-08

**Status:** Approved for implementation

## Goal

Reduce repeated filesystem, configuration, and static-resource work in CatDesk while preserving exact final `changedFiles` reporting and existing command behavior.

## Scope

The implementation covers the high-confidence backend costs identified in `docs/PERFORMANCE.md`:

- repeated change snapshots caused by `poll_command`;
- repeated configuration and `AGENTS.md` reads within widget/instruction paths;
- repeated construction of identical changed-file payloads;
- repeated encoding of static widget PNGs;
- repeated command-availability probes where the cache can preserve the existing fallback behavior.

The implementation deliberately does not change TUI redraw scheduling, token-count approximation, or remote-debug startup polling. Those changes affect user-visible timing or display behavior and are lower priority than preserving correctness.

## Correctness Invariants

1. A terminal command job's `changedFiles` result is computed from a complete after-snapshot and cannot be based solely on mtime or file size.
2. `run_command`, `write`, `edit`, and `delete` retain their existing exact before/after snapshot behavior.
3. Configuration and instruction caches invalidate when the relevant file metadata changes; read/parse errors are not cached permanently.
4. Existing MCP response fields, widget payload fields, search fallback behavior, and token counts remain unchanged.
5. An in-progress `poll_command` may omit intermediate file changes. The terminal response is the authoritative complete change report.

## Design

### 1. Terminal-only command-job change collection

`start_command` already creates a `ChangeSession` before spawning the process. The job will additionally own a one-time final change result. `CommandJobManager::current_changes` will:

- return no changes while the job is still running;
- when the job is terminal, run `ChangeSession::changes` once using the complete filesystem snapshot;
- cache that result and return clones on later polls, including polls used only to drain buffered output.

This removes the repeated recursive scan from long-running `poll_command` calls without weakening the final report. No mtime-only shortcut is used for the terminal scan.

### 2. Configuration and instruction reuse

The MCP module will use metadata-keyed caches for the application config and resolved instruction text. The cache key includes the canonical path, file length, and modification time. Cache misses read and parse normally; failures remain retryable. `catdesk_instruction` will compute its instruction text once and reuse it for the plain response and structured response.

The widget payload will reuse the cached config/agent state. The archived Binagotchy directory remains a cold-path read and is not globally cached, avoiding stale nested archive data.

### 3. Payload and static-resource reuse

Changed-file entries will be produced through one shared conversion path so structured content and widget payload construction cannot drift. Static PNG bytes remain compile-time assets, while their base64 strings are initialized once per process and reused by `render_widget_html`.

### 4. Search backend probe

Availability probes for `rg` and `grep` will be cached per process. If a selected backend disappears while executing, the existing backend error/fallback handling remains authoritative; the cache only removes repeated `--version` probes.

## Testing

Add focused tests for:

- a running command returning without a filesystem diff scan and a terminal command returning the complete diff;
- repeated terminal polls reusing the same final changes;
- changed-file correctness for create, modify, delete, same-size edits, binary files, symlinks, and nested paths;
- config and `AGENTS.md` cache invalidation after file updates;
- unchanged widget/resource payload fields and PNG rendering;
- search backend selection and fallback behavior.

Run the repository gates after each implementation batch:

```bash
cargo fmt --check
cargo test --release
cargo build --release
```

## Rollback Boundaries

Keep change tracking, config/instruction caching, and static-resource/payload changes in separate commits. If any correctness test or manual smoke test regresses, revert that subsystem commit without affecting the others.
