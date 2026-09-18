# CatDesk Upstream v0.9.1 Integration Design

## Goal

Integrate the useful upstream changes through commit `0f549713228e2335031f98b870916fc642dfbd5c` (v0.9.1) into the fork while preserving the fork-specific Cloudflare/external tunnel, Landlock fallback, performance hardening, startup hardening, and security behavior.

## Constraints

- Upstream is read-only. Never push to `upstream`.
- Keep `public_base_url` and external HTTPS tunnel support. Do not restore the upstream ngrok lifecycle or `src/ngrok.rs`.
- Keep the current Bubblewrap + Landlock fail-closed fallback model.
- Keep fork-specific performance caches, deferred change scanning, non-blocking startup, DevTools hardening, and workspace/symlink hardening.
- Prefer upstream v0.9.1 behavior for Git worktree/external Git metadata validation, sandbox process lifetime, local log time, and ChatGPT-aligned widget behavior where it does not conflict with the constraints above.
- The final branch must pass `cargo fmt --check`, `cargo test --release`, and `cargo build --release`.

## Integration Strategy

The fork's v0.8.0 integration was squash-like rather than a true Git merge, so a normal merge produces misleading ancestry and high conflict volume. Integrate at the feature level instead of merging `upstream/main` wholesale.

### Sandbox and process lifetime

Port the v0.9.1 Git metadata/worktree validation from upstream while retaining the fork's Landlock fallback. Port `dac5803` so Bubblewrap-backed commands are prepared in `spawn_blocking` but the actual process is spawned from a Tokio runtime worker, avoiding premature process death caused by `--die-with-parent` and retired blocking threads.

### Local log time

Adopt upstream local-time formatting (`local_now`) and the related state/log behavior without restoring unrelated ngrok state.

### Widget and TUI

Adopt upstream ChatGPT-aligned widget surfaces, theme fixes, and corner-style setting. Adapt TUI settings reachability so the fork exposes `public_base_url` in the slot where upstream exposes ngrok domain configuration. Do not add ngrok dependencies or runtime state.

### Version and release metadata

After successful integration, update crate/package versions to `0.9.1` so runtime/package metadata matches the upstream feature baseline. Regenerate `Cargo.lock` through Cargo rather than hand-editing it.

## Verification

Add or preserve focused regression tests for process lifetime, Git worktree metadata validation, local-time behavior, widget corner/theme output, and public URL configuration. Run the complete release test and build commands before considering the integration complete.
