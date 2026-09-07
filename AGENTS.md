# Repository Guidelines

## Project Structure

CatDesk is a Rust MCP server with a small Node.js launcher. Application code is in `src/`; major areas include MCP/API handling (`mcp.rs`, `server.rs`), workspace tools (`workspace_tools.rs`), command execution (`command*.rs`), platform integrations, and UI assets under `src/widget/`. Unit tests live beside the Rust modules. Fixtures are in `tests/fixtures/`, packaging helpers are in `npm/`, release automation is in `.github/workflows/`, and user-facing documentation is in `README.md` and `CONTRIBUTING.md`.

## Build, Test, and Development Commands

- `cargo fmt` formats Rust sources; use `cargo fmt --check` in CI-style checks.
- `cargo test --release` runs the Rust unit and async tests.
- `cargo build --release` builds the optimized binary at `target/release/catdesk`.
- `cargo run --release` runs CatDesk locally from the current workspace.
- `npm install -g .` installs the Node launcher and invokes the packaged binary workflow.

Keep `Cargo.lock` committed. Release tags must match the versions in both `Cargo.toml` and `package.json`; the release workflow validates this automatically.

## Coding Style and Naming

Use standard `rustfmt` output with four-space indentation. Follow Rust naming conventions: `snake_case` for functions, modules, and variables; `UpperCamelCase` for types; and `SCREAMING_SNAKE_CASE` for constants. Keep platform-specific code in the existing platform modules and prefer small, explicit changes over new abstractions.

## Testing Guidelines

Add focused tests next to the code they exercise using `#[test]` or `#[tokio::test]`. Name tests for observable behavior (for example, `tracks_bootstrap_completion`). Run `cargo fmt --check`, `cargo test --release`, and `cargo build --release` before opening a pull request. No coverage threshold is currently enforced.

## Commits and Pull Requests

Use concise Conventional Commit subjects such as `feat:`, `fix:`, `docs:`, `refactor:`, or `test:`. Pull requests target `main` and should explain the user-visible change, note platform or configuration impact, and include screenshots or command output when UI or release behavior changes. Keep unrelated cleanup out of the same PR.

## Security and Fork Workflow

CatDesk can execute commands and modify files; test it in a VM or container and never commit tokens or local configuration. For forks, keep `upstream` pointed at `Xeift/CatDesk` for fetches and push only to your fork's `origin`; do not push directly to the upstream repository.

## Agent skills

### Issue tracker

Issues and specs are tracked in GitHub Issues at `Xeift/CatDesk`. See `docs/agents/issue-tracker.md`.

### Triage labels

The default five-role triage vocabulary is used. See `docs/agents/triage-labels.md`.

### Domain docs

This repository uses a single-context layout. See `docs/agents/domain.md`.
