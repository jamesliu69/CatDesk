# CatDesk 速度效能改進報告

> 產生日期：2026-09-08
> 範圍：本地 fork（基於 upstream `Xeift/CatDesk` main `bfac47b` 之後的本地修改）
> 性質：唯讀分析報告，未含任何程式碼變更

## 總結

MCP 工具路徑（`poll_command` 長輪詢）已經是 event-driven（`Notify` based），沒有油水可榨。真正的效能浪費集中在兩處：

1. **每次 tool call 的重複計算**（snapshot diff、config 重讀、重複序列化）
2. **TUI 60fps 無條件重繪**（idle 時 CPU 全餵這裡）

---

## 高收益（伺服器端，影響每個 tool call）

### 1. Snapshot diff：每次改動 tool call 跑兩次全目錄掃描 ⭐ 最大

- `ChangeSession::begin`（`src/mcp.rs:1090`）在工具執行**前**做快照；`.changes()`（`src/mcp.rs:1176`）執行**後**再做一次快照 + diff。
- 快照會讀**檔案內容**（每檔上限 128 KB、最多 512 檔，`src/change_tracking/snapshot.rs:233`）——不是只看 mtime。
- 最慘的路徑：`run_command` / `start_command` 的 scope 是 `discovered(cwd, true)`，遞迴整個 cwd 子樹，且**每次 `poll_command` 都重掃一次**。輪詢一個 dev server = 每次輪詢全樹重快照 + diff。
- 工具執行失敗時，after-snapshot 仍然照跑（白掃一次）。

**改法**：
- digest-first：stat + mtime 沒變就跳過內容讀取，只有變過的檔案才讀內容做 diff。
- failed call 跳過 after-snapshot。
- poll 路徑沿用 job 既有的 change session，不在每次 poll 重掃。

### 2. `load_app_config()` 每次 tool call 都重讀磁碟

- 呼叫鏈：`base_widget_payload` → `current_token_stats_layout()`（`src/mcp.rs:2642`）→ `load_app_config()`（`src/state.rs:594`）→ fs read + TOML parse。
- 沒有任何快取，每次都打磁碟。

**改法**：mtime 快取，一個 helper 修掉全部 5+ 個呼叫點。

### 3. `catdesk_instruction` 一次呼叫做三遍同樣的工作

- `catdesk_instruction_text` 被呼叫**兩次**（`src/mcp.rs:2243` 直接一次 + `catdesk_instruction_structured` 內 `src/mcp.rs:2158` 再一次），每次都重新解析 AGENTS.md（`load_app_config` + 3 個 `is_file()` 探測 + fs read）。
- widget payload 再加第三遍（`agents_widget_state_payload`，`src/mcp.rs:2184`）。
- 另外每次呼叫都掃 `~/.catdesk/binagotchy` 封存目錄 + 重建 mascot JSON（`src/mcp.rs:2213`、`src/mcp.rs:2231`）。

**改法**：AGENTS.md 解析結果算一次傳給三個 sink；binagotchy 封存清單啟動時掃一次快取。

### 4. `changedFiles` 序列化兩次

- 同一份 FileChange 資料進 `structuredContent`（`src/mcp.rs:1206` / `2568`）**又**進 `_meta` widget payload（`src/mcp.rs:2622-2633`），各自做一次 JSON 序列化。

**改法**：序列化成 `serde_json::Value` 一次，兩處共用。

### 5. `search` 每次先 spawn `rg --version`

- `command_available("rg")`（`src/workspace_tools.rs:736` → `765-772`）每個 search call 都開一個 process 探測，再開真正的 rg。rg 不存在時還要再探測一次 grep。

**改法**：啟動時探測一次，快取成 bool。

---

## 中收益

### 6. Token 估算多重序列化

- 每個 widget-enabled call 把結果 `serde_json::to_string` 再跑 o200k encode（`src/mcp.rs:2298-2310`），輸出大時 encode 成本線性增長；回應本身之後還要再序列化一次。
- tokenizer 已是 singleton（好的）。

**改法**：只估 head / 取樣，或延後到真正要顯示時才算。看 profiling 數字再決定。

### 7. `render_widget_html` 每次重編 3 張 PNG 的 base64

- 靜態資源每次 `resources/read` 都重新 base64 encode（`src/mcp.rs:369-380`）。

**改法**：`OnceLock` 編譯期常數算一次。冷路徑，優先度低。

---

## TUI 端（本機 CPU，不影響 LLM 速度）

### 8. `run_tui` 無條件 60fps 重繪 ⭐

- `src/main.rs:3373-3466`：每 16.7ms 就 `terminal.draw` + 全螢幕字串重建（`3438-3444`）+ animation snapshot，**沒有任何事件也照畫**。idle 時 CPU 消耗全部來自這裡。
- 同模式複製到 `run_app`、`run_settings`、`run_browser_select` 等 loop。

**改法**：dirty-flag（沒變就不重繪），或 `tokio::select!` + crossterm `EventStream` 轉全 event-driven。

### 9. `run_prompt` 100ms 輪詢

- `src/main.rs:2053`：輸入延遲最高 100ms，是所有 loop 裡最差的互動延遲。

**改法**：同第 8 項修法。

---

## 已驗證健康（不用動）

| 項目 | 位置 | 狀態 |
|---|---|---|
| Build profile：lto + codegen-units=1 + strip | `Cargo.toml:40-43` | ✅ 已最佳化 |
| `poll_command` MCP 長輪詢 | `src/command_jobs.rs:391-400` | ✅ Notify-based，零延遲喚醒 |
| `cancel_command` | `src/command_jobs.rs:427-444` | ✅ event-driven |
| `read` 工具 multi-read + symlink/重複去重 | `src/workspace_tools.rs:401, 424-456` | ✅ 無單讀殘留 |
| Git 操作 | — | ✅ 無 subprocess，PR #23 已改 in-process `similar` diff |
| Widget HTML 資源 | `src/mcp.rs:33` | ✅ static `include_str!` |
| server.rs 的 config.toml 讀取 | `src/server.rs` 11 處 | ✅ 全在 `#[cfg(test)]`，非 per-request |
| Widget 端 `setTimeout` | widget HTML | ✅ 一次性動畫 timer，非輪詢 |
| `wait_remote_debug_ready` 250ms poll | `src/main.rs:3030-3048` | ⚠️ browser 模式啟動路徑，僅加最多 250ms，低價值 |
| macOS `thread::sleep` | `src/macos_terminal.rs:147,156,174` | ⚠️ 僅啟動時一次性，block thread 但無 runtime 影響 |

---

## 已知但改不了的

| 項目 | 說明 |
|---|---|
| 50+ tool call 後 ChatGPT 變慢（~3.9 GB RAM） | ChatGPT 客戶端問題，CatDesk 端無解。README 已建議開新 session + handoff note |
| Startup intro 固定 1.85 秒 | 產品決策（`src/startup.rs`，按鍵可跳過） |
| LLM round-trip 延遲 | 「少打 tool call」（如 PR #23）的效益遠大於任何伺服器端優化——那才是延遲大宗 |

---

## 建議實作順序

| 優先 | 項目 | 理由 |
|---|---|---|
| 1 | #1 digest-first 快照 | 最大節省；run_command/poll 路徑的掃描成本直接砍一個數量級 |
| 2 | #8 TUI dirty-flag | idle CPU 歸零 |
| 3 | #2 + #3 config / AGENTS.md 快取 | 一個 mtime helper 解掉兩項 |
| 4 | #4 + #5 序列化共用 / rg 探測快取 | 各自一行級改動 |
| 5 | #6 / #7 / #9 | 先量 profiling，有數字再動 |

## 量測建議

- #1：`start_command` 起 dev server 後連續 `poll_command` 10 次，比較改前/改後總耗時與 CPU。
- #8：idle 狀態下看 process CPU%（目前預期持續非零）。
- #2/#3：`catdesk_instruction` 連打 10 次的延遲。
