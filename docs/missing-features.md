# CatDesk 缺失功能調查報告

**調查日期**：2026-08-31
**調查範圍**：`Xeift/CatDesk`（本地工作區 `d:\Repo\CatDesk`，基於 `feat/cloudflare-public-base-url` 分支，HEAD `e6ef92b`）
**性質**：研究報告，僅記錄現狀，不包含實作。

---

## 一、方法與資料來源

| 來源 | 用途 |
| --- | --- |
| `README.md`、`README.zh-TW.md` | 專案自我宣稱的功能範圍、限制、Similar projects 清單 |
| `docs/TECHNOLOGY.md` | 架構與模組職責 |
| `CONTRIBUTING.md`、`AGENTS.md` | 開發與倉庫慣例 |
| `CATDESK_PI_LANDLOCK_FIX.md` | 已知平台限制案例（Raspberry Pi Landlock） |
| `src/` 原始碼（以 `mcp.rs`、`command.rs`、`command_jobs.rs`、`workspace_tools.rs`、`browser.rs`、`devtools.rs`、`server.rs`、`state.rs`、`linux_sandbox.rs`、`process_runner.rs` 為主） | 實際工具清單、上限常數、未完成標記 |
| GitHub Issues / PR（`gh` CLI，已登入） | 功能需求一手來源 |
| 相似專案的 README 原文（GitHub raw 內容） | 對照能力缺口 |

引用格式：（來源：`路徑:行號`）或（來源：issue #N / PR #N）。

---

## 二、目前功能盤點（MCP 工具清單）

README 宣稱 `multi-tools` 模式提供 10 個工具、`read-only` 模式提供 3 個工具（來源：`README.md:213`），與原始碼一致。

### 本機工具（`multi-tools` 模式，10 個）

| 工具 | 功能 | 定義位置 |
| --- | --- | --- |
| `catdesk_instruction` | 回傳使用規則並渲染 Binagotchy；其他工具必須先成功呼叫它一次（來源：`src/mcp.rs:218-223` 強制檢查、`src/mcp.rs:724`） | `src/mcp.rs:723-732` |
| `read` | 一次讀多個文字檔（最多 32 檔、整批 512 KB）（來源：`src/workspace_tools.rs:15-16`） | `src/mcp.rs:849-871` |
| `search` | 以 rg → grep → 內建掃描的順序搜尋 workspace（來源：`src/workspace_tools.rs:693-718`） | `src/mcp.rs:872-897` |
| `write` | 建立或覆寫檔案，可建立父目錄 | `src/mcp.rs:898-912` |
| `edit` | 原子化 guarded replace/range 編輯 | `src/mcp.rs:913-955` |
| `delete` | 刪除檔案或目錄 | `src/mcp.rs:956-968` |
| `run_command` | 短 shell 命令，最長 120 秒（來源：`src/command.rs:7`） | `src/mcp.rs:764-787` |
| `start_command` | 啟動長時間命令並回傳 job ID（最長 24 小時，來源：`src/command_jobs.rs:17`） | `src/mcp.rs:787-811` |
| `poll_command` | 增量讀取背景命令輸出與狀態（單次輪詢等待最長 30 秒，來源：`src/command_jobs.rs:18`） | `src/mcp.rs:811-833` |
| `cancel_command` | 取消背景命令及其子程序樹（Unix process group / Windows Job Object，來源：`src/process_runner.rs:305-320`、`330-345`） | `src/mcp.rs:833-848` |

### 本機工具（`read-only` 模式，3 個）

僅 `catdesk_instruction`、`read`、`search`（測試斷言見來源：`src/mcp.rs:4574`；模式定義見來源：`src/state.rs:462-499`）。

### 瀏覽器工具

由外部 `npx -y chrome-devtools-mcp@latest` 子程序提供，透過 stdio JSON-RPC bridge 轉發，實際清單依環境而定（來源：`src/devtools.rs:22-50`）。瀏覽器偵測與 Remote Debugging 啟動在 `src/browser.rs`（Remote Debug port 9222–9322，來源：`src/main.rs:3071`）。

### 支援的 MCP 方法

`server/discover`、`tools/list`、`tools/call`、`resources/list`、`resources/read`、`ping`（來源：`src/mcp.rs:165-200`）；協定版本 `2026-07-28`（來源：`src/mcp.rs:27`）。HTTP 層另外驗證 `MCP-Protocol-Version`、`Mcp-Method`、`Mcp-Name` header 與 `_meta`（來源：`src/server.rs:181-300`）。

### 各模組職責（簡述）

| 模組 | 職責 |
| --- | --- |
| `src/change_tracking/` | 命令前後拍 snapshot 產生 unified diff，避開 `.git`、尊重 ignore 規則；上限 16 檔／每檔 12,000 字元／追蹤 512 條目（來源：`src/change_tracking/mod.rs:9-12`） |
| `src/theme/` | TUI 配色（`concise`、`neon` 兩套主題定義，來源：`src/theme/mod.rs:8-48`） |
| `src/widget/` | ChatGPT 內嵌 Widget（單一 HTML 檔，原生 JS，來源：`docs/TECHNOLOGY.md` 第 259 行附近） |
| `src/binagotchy_gen/` | 純程式碼產生鯊魚貓像素角色（PNG/GIF/TUI/Widget 動畫，來源：`docs/TECHNOLOGY.md` 結尾章節） |
| `src/command.rs` | 路徑解析、tree-sitter-bash 解析與 `find`/`tree`/`ls -R`/`rg --files`/`mv` 攔截、git commit Co-Authored-By trailer 注入（來源：`src/command.rs:244`） |
| `src/linux_sandbox.rs` | Landlock ABI v3 檔案系統隔離（來源：`src/linux_sandbox.rs:20`） |
| `src/macos_terminal.rs` | macOS Terminal.app 專用 profile 的詢問與套用 |

---

## 三、缺失／未完成功能清單

### (a) 程式碼內標記未完成（TODO 等）

**結論：`src/` 內沒有任何 `TODO`、`FIXME`、`unimplemented`、`not implemented`、`todo!` 標記。**（grep 全 `src/**` 無結果，2026-08-31 檢查）

程式碼中明確標示「尚未支援」的功能只有一處：

1. **Firefox 瀏覽器控制未接線** — Firefox 被偵測但標記為 `mcp_supported: false`（來源：`src/browser.rs:106-111`，支援註記寫明 "Not supported yet (CDP bridge for Firefox not wired)"）；`docs/TECHNOLOGY.md:243` 同样說明「尚未接好 Firefox 的 CDP bridge」。

### (b) GitHub Issues 與開放 PR 上的功能需求

`Xeift/CatDesk` 目前僅 4 個 issue（1 開放／3 關閉）與 4 個 PR（2 開放／2 關閉）（`gh` CLI 查詢，2026-08-31）。開放項目全部與功能缺口相關：

1. **PR #14 — 支援 Ubuntu 22.04 以外的 Linux 發行版**（開放）
   - 三個獨立 blocker（來源：PR #14 描述原文）：
     - 官方 binary 在 RHEL 8 / Rocky 8 / CentOS 8 無法啟動：需要 `libssl.so.3` 與 glibc 2.34（在 ubuntu-22.04 上建構所致）；
     - Control Computer 模式在 kernel < 5.13（無 Landlock）完全不可用；
     - panic 時沒有終端機 teardown hook，shell 會壞掉直到 `reset`。
   - PR 建議解法：reqwest 改 rustls、以 bubblewrap `-b` 作為 sandbox fallback、加 panic hook。截至調查日尚未合併。
   - 佐證：`Cargo.toml:14` 目前確實是 `reqwest = { version = "0.12", features = ["json"] }`（預設 native-tls → OpenSSL）；`src/linux_sandbox.rs:165-180` 對非 FullyEnforced 一律拒絕，需 `CATDESK_ALLOW_UNSANDBOXED_LINUX=1` 才放行（來源：`src/linux_sandbox.rs:15`）。

2. **PR #3 — 外部 MCP gateway 與 skills 工具**（開放，未合併）
   - 提議讓 CatDesk 作為 gateway 掛載其他外部 MCP server（`~/.catdesk/config.toml` 內設定），並提供本地 skill 發現工具。`src/` 內沒有任何 gateway 實作（grep `mcp.mcpServers`、`toolPrefix` 無結果），即目前不存在此功能。

3. **PR #13 — ChatGPT reasoning profiles**（開放，未合併）
   - 提議把使用者選的 reasoning 等級存進 config，讓 `catdesk_instruction` 的引導文字隨之調整。`src/state.rs` 目前無此設定欄位（grep `reasoning` 無結果）。

4. **issue #12 — 在 README 連結社群雙語指南**（開放）
   - 外部維護的入門指南（andrewcodehappily.github.io/catdesk-guide）希望被加入 README。屬於 docs 整合需求。

5. **issue #1（已關閉）— 權限自動授予討論**
   - 使用者厭倦反覆點 ChatGPT 的權限核可，貢獻了 Greasemonkey 使用者腳本 workaround。反映出「ChatGPT 端權限彈窗無法由 CatDesk 側消除」的缺口；該討論已關閉、官方未內建任何對應機制（`src/` 無自動核可相關程式）。

6. **issue #5（已關閉）— ngrok token 導致 crash**
   - 已透過移除內建 ngrok 解決（commit `58d0a53`「移除內建 ngrok 串接，改用外部 HTTPS tunnel 與 public_base_url 設定」）。屬已修復歷史，但留下了「CatDesk 不再內建任何 tunnel 功能」的現狀。

### (c) 與相似專案對照的能力缺口

README 列出 7 個相似專案（來源：`README.md:72-84`）。以下對照只使用對方 README 自我描述與 CatDesk 原始碼「找不到」的證據：

1. **Git 專用工具集**
   - ChatGPT Local Coder 提供 `git_status` / `git_diff` / `git_log` / `git_add` / `git_commit` / `git_branch` / `git_checkout` / `git_restore` / `git_push` / `git_pull` / `git_stash` / `git_reset`（來源：ChatGPT Local Coder README「Git」段落）。
   - CatDesk 沒有任何 `git_*` 工具（`tools/list` 定義僅上表 10 個，來源：`src/mcp.rs:723-968`），只有 `run_command` 攔截 `git commit` 注入 Co-Authored-By trailer（來源：`src/command.rs:238-244`）與 instruction 文字內的 git 建議（來源：`src/mcp.rs:2093-2094`）。Git 操作完全依賴模型自行下 shell 命令。

2. **apply_patch / unified diff 直接套用**
   - ChatGPT Local Coder 提供 `apply_patch`（Codex 風格 patch，來源：其 README「Filesystem」段落）。
   - CatDesk 的 `edit` 僅支援 `replace` 與 `range` 兩種操作（來源：`src/mcp.rs:913-955` 的 inputSchema oneOf 只有兩個分支），無 patch 格式支援。

3. **互動式程序與持續 shell session**
   - Desktop Commander 提供 `interact_with_process`（對 SSH、REPL、dev server 送輸入並讀回應）、`read_process_output`、Session 管理（來源：Desktop CommanderREADME「Available Tools」表格）。
   - CatDesk 的命令一律無互動：Windows PowerShell 用 `-NonInteractive`（來源：`src/process_runner.rs:337-344`），程序建立後 stdin 直接關閉（來源：`src/process_runner.rs:303`）。`start_command`/`poll_command` 只能讀輸出，不能寫入。grep `pty`、`interact` 在 `src/` 無結果。

4. **系統程序管理（list / kill 任意程序）**
   - Desktop Commander 提供 `list_processes`、`kill_process`（來源：其 README 表格）。
   - CatDesk 只有自己啟動的 8 個背景 job 生命週期管理（並行上限 8、保留上限 64，來源：`src/command_jobs.rs:20-21`），沒有列出或終止系統上其他程序的工具。

5. **二進位檔案讀寫**
   - ChatGPT Local Coder 提供 `read_file_base64` / `write_file_base64`（來源：其 README「Filesystem」段落）。
   - CatDesk `read` 以 `String::from_utf8_lossy` 讀檔（來源：`src/workspace_tools.rs:279`），工具描述與 schema 均限文字檔；無 base64 或二進位工具（`src/mcp.rs` 工具定義中無 binary 相關參數）。

6. **Office / PDF 原生支援**
   - Desktop Commander 宣稱原生讀寫 Excel（.xlsx/.xls/.xlsm）、PDF 文字擷取與建立、DOCX 讀寫搜尋（來源：其 README「Features」段落）。
   - CatDesk 完全沒有這類工具（grep `excel|pdf|docx` 在 `src/mcp.rs` 工具註冊處無結果），需靠模型在 shell 內自行處理。

7. **細粒度權限／允許清單機制**
   - CodexPro 自我描述「scoped to explicitly allowed repositories」（來源：`README.md:80`）；Desktop Commander 有 `allowedDirectories`、`blockedCommands` 設定（來源：其 README「Available Tools — get_config」列）。
   - CatDesk 的邊界只有：workspace root 路徑檢查（canonicalize 後須位於 workspace 內，來源：`docs/TECHNOLOGY.md` Workspace 段）與 `ReadOnly`／`MultiTools` 兩檔工具模式（來源：`src/state.rs:462-499`）。沒有 per-tool 允許清單、命令黑名單或多 workspace 設定（grep `allowlist|allow_list|whitelist|blockedCommands` 在 `src/` 無結果）。

8. **端點認證**
   - ChatGPT Local Coder 支援 `MCP_TOKEN` 路徑 token（來源：其 README「Security」段）。
   - CatDesk 的 MCP URL 僅靠首次啟動的 random secret path 保護，官方技術文件明言「目前 MCP URL 沒有額外登入驗證」（來源：`docs/TECHNOLOGY.md:291`；widget 行為路由也僅用同一個 secret prefix，來源：`src/server.rs:55-64`）。`src/server.rs` 內沒有 Authorization header 驗證（grep `auth` 僅命中測試的無關字串）。

9. **MCP client 相容廣度**
   - Desktop Commander 支援 Claude Desktop、Cursor、Windsurf、VS Code、Cline、Roo、Codex CLI、Gemini CLI 等十餘種 client 與遠端 MCP 服務（來源：其 README「Install in Other Clients」）。
   - CatDesk 明言專為 ChatGPT Custom Connector 設計與測試，其他 app「可能不順」（來源：`README.md` FAQ "Can CatDesk be used in other apps?"）；且 GET SSE 串流被刻意停用，僅支援 pure HTTP 模式（來源：`src/server.rs:3146`：「GET SSE stream is disabled in pure HTTP mode」）。這對需要 SSE/STREAMABLE HTTP session 的 MCP client 是硬缺口。

10. **程序輸出分頁讀取等細節差異（CatDesk 已有的對照項，非缺口）**
    - CatDesk 的 `search`／`read`／背景 job 輸出 cursor 機制大致對齊相似專案，此列表僅列缺口，不重複。

### (d) 平台與剛性限制

1. **必須自備外部 HTTPS tunnel** — CatDesk 不再啟動或管理任何 tunnel，需先設定 Cloudflare Tunnel 等外部服務並輸入 public base URL（來源：`README.md:115`；`src/main.rs:1708` 的提示文字）。斷線重啟、DNS、systemd 全由使用者維運。

2. **安全模型建議整機隔離** — 免責聲明與 Quickstart 都強烈建議在 VM／容器內執行（來源：`README.md:14`、`README.md:96-99` CAUTION 區塊）。

3. **Linux 沙盒依賴 Landlock（kernel ≥ 5.13, ABI v3）** — Landlock 完全不可用時預設拒絕執行命令，需明確設定 `CATDESK_ALLOW_UNSANDBOXED_LINUX=1`；部分生效一律拒絕（來源：`src/linux_sandbox.rs:15-20`、`165-180`；`CATDESK_PI_LANDLOCK_FIX.md` 記錄 Raspberry Pi OS 無 `CONFIG_SECURITY_LANDLOCK` 的實例與該 opt-in 變數）。

4. **預編譯 binary 的 glibc/OpenSSL 下限** — 官方發佈 binary 依 PR #14 驗證無法在 RHEL 8 / Rocky 8 / CentOS 8（glibc 2.28 + OpenSSL 1.1.1）啟動（來源：PR #14）；目前 `Cargo.toml:14` 仍使用 reqwest 預設 TLS 後端。

5. **瀏覽器控制僅限 Chromium 系** — Firefox 不可用（來源：`src/browser.rs:106-111`）；chrome-devtools-mcp 需要 Node.js/npx 存在（來源：`src/devtools.rs:26-27`）。

6. **MCP URL 無認證、持有即掌控** — 任何人取得完整 URL 即可存取電腦，官方以防呆警告代替技術防護（來源：`README.md:342-345` CAUTION、`docs/TECHNOLOGY.md:291`）。

7. **`read-only` 與否實際由 ChatGPT 端決定** — connector 的權限（Allow read actions / Allow all actions）是 ChatGPT 設定，CatDesk 側僅能提供工具註記；全部放開等同 Codex `--yolo`（來源：`README.md` Quickstart 步驟 7）。

8. **ChatGPT 平台端的既有問題（CatDesk 無法修）** —
   - 50+ 次工具呼叫後極度卡頓、記憶體飆高，官方建議每個小功能開新 session（來源：`README.md:176-178`）；
   - 改 MCP 設定後需重裝 connector（來源：`README.md:182`）；
   - widget 偶發空白是 ChatGPT 端 bug，官方明言「無能為力」（來源：`README.md` FAQ "What to do if the widget is blank?"）；
   - ChatGPT Web context window 較小（Plus 128K+128K=256K，來源：`README.md:236-243`）。

9. **資源上限** — `run_command` 120 秒上限（來源：`src/command.rs:7`）；背景 job 最多 8 個並行、保留 64 個、每 job 4 MB 輸出、單次 poll 128 KB（來源：`src/command_jobs.rs:20-26`）；讀檔整批 512 KB／32 檔（來源：`src/workspace_tools.rs:15-16`）；diff 追蹤 16 檔/512 條目（來源：`src/change_tracking/mod.rs:9-12`）。

10. **Token 統計僅為本地估算** — CatDesk 拿不到 ChatGPT 官方數字，以 `o200k_base` 估算工具輸入輸出，不含對話本體與 reasoning tokens（來源：`README.md:291-296`）。

---

## 四、建議優先順序（個人判斷，僅供參考）

1. **修 PR #14 涵蓋的三大 Linux blocker**（glibc/OpenSSL 相容、Non-Landlock fallback、panic teardown）— 直接決定「官方 binary 能不能跑」，影響所有非 Ubuntu 22.04 / 非 kernel 5.13+ 使用者。
2. **端點認證（token 或 HMAC）** — 現況「URL 即憑證」對必經公網 tunnel 的架構風險過高，且相似專案已有現成模式。
3. **合併或重新評估 PR #3（MCP gateway）** — 一次補齊與其他 MCP server 生態的整合缺口，從根源擴大能力不需逐一加工具。
4. **Git 工具集與 `apply_patch`** — 對「把 ChatGPT 當編碼代理」的核心場景加值最大，且 CatDesk 已有 change_tracking/diff 基礎。
5. **互動式程序支援（`send_input` 類工具）** — 補齊 dev server／REPL 工作流；工作量較大，可後置。
6. **Firefox CDP bridge** — 使用者族群較小，優先度最低。

---

## 五、來源清單

### 本地檔案

- `README.md`、`README.zh-TW.md`、`CONTRIBUTING.md`、`AGENTS.md`
- `docs/TECHNOLOGY.md`
- `CATDESK_PI_LANDLOCK_FIX.md`
- `src/mcp.rs`、`src/server.rs`、`src/state.rs`、`src/command.rs`、`src/command_jobs.rs`、`src/process_runner.rs`、`src/workspace_tools.rs`、`src/browser.rs`、`src/devtools.rs`、`src/linux_sandbox.rs`、`src/macos_terminal.rs`、`src/main.rs`、`src/change_tracking/mod.rs`、`src/theme/mod.rs`
- `Cargo.toml`

### GitHub（Xeift/CatDesk，經 `gh` CLI 取得原文）

- issue #12（開放）、issue #1、#5、#7（已關閉）
- PR #14、PR #3、PR #13（開放）；PR #11、#10、#9、#8、#6、#4、#2（已關閉）

### 相似專案（README 原文）

- Desktop Commander MCP（GitHub 儲存庫 wonderwhy-er/DesktopCommanderMCP，讀取其 README）

- ChatGPT Local Coder（GitHub 儲存庫 hoangcoderr/chatgpt-local-coder，讀取其 README）

**調查受阻部分**：無。`gh` CLI 已登入可用，issue/PR 全數讀取原文；`src/` 的 TODO 掃描與工具清單均直接檢查原始碼。相似專案對照僅使用其 README（依任務要求未深入程式碼），其功能描述以對方自我宣稱為準。
