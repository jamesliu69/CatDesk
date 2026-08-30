# CatDesk 技術說明

## 一句話理解

CatDesk 是一個用 Rust 開發的本機電腦控制橋接器。它讓 ChatGPT Web 透過 MCP（Model Context Protocol）讀寫指定 Workspace 的檔案、執行 Shell 指令，並透過 Chromium DevTools 控制瀏覽器。

CatDesk 本身不是 AI 模型，也不呼叫 OpenAI API。ChatGPT Web 負責理解需求與決定要呼叫什麼工具，CatDesk 負責在本機執行這些工具。

## 整體架構

```text
ChatGPT Web
    │ MCP / JSON-RPC over HTTPS
    ▼
ngrok 公開 Tunnel
    │ 轉送到 127.0.0.1:3200
    ▼
Rust + Axum HTTP Server
    │
    ├─ MCP 工具分派器
    │    ├─ 讀寫檔案
    │    ├─ 搜尋文字
    │    ├─ 執行 Shell 指令
    │    └─ 管理背景工作
    │
    ├─ chrome-devtools-mcp
    │    └─ 操作本機 Chromium 瀏覽器
    │
    ├─ Widget HTML/JavaScript
    │    └─ 在 ChatGPT 中顯示狀態與 Diff
    │
    └─ Ratatui TUI
         └─ 終端機控制台、日誌與動畫
```

## 技術棧

| 領域 | 技術 | 白話說明 |
|---|---|---|
| 主要語言 | Rust 2024 Edition | 編譯成原生執行檔，速度快且有記憶體安全保護 |
| 非同步執行 | Tokio | 同時處理 HTTP、Shell、背景工作與 UI 事件 |
| HTTP Server | Axum | 提供本機 MCP HTTP API |
| MCP | 自行實作 JSON-RPC | 不依賴 MCP SDK，直接處理協定訊息 |
| 資料格式 | Serde、serde_json、toml | 解析 JSON、MCP 訊息與設定檔 |
| 終端機 UI | Ratatui、Crossterm | 顯示彩色 TUI、鍵盤、滑鼠與動畫 |
| 公開連線 | ngrok Rust SDK | 把本機 Server 暫時公開到網際網路 |
| 瀏覽器控制 | chrome-devtools-mcp | 透過 Chromium Remote Debugging 控制瀏覽器 |
| 檔案搜尋 | ripgrep、grep、內建 Rust 搜尋 | 依環境自動選擇搜尋後端 |
| Shell 解析 | tree-sitter-bash | 解析 Bash 指令，而非單純切割字串 |
| Diff | similar | 計算修改前後的 unified diff |
| Linux 安全 | Landlock | 限制 Shell 只能存取允許的目錄 |
| Windows 程序管理 | Windows Job Object | 結束命令時連同子程序一起結束 |
| 圖像 | image、rand | 程式化產生 Binagotchy 像素角色 |
| 發佈 | npm + 預編譯 Rust binary | 使用 `npm install -g catdesk` 安裝 |

完整依賴位於 [`Cargo.toml`](../Cargo.toml)。README 的原始 Stack 表位於 [`README.md`](../README.md#stack)。

## 啟動流程

入口位於 [`src/main.rs`](../src/main.rs)。啟動時會：

1. 讀取 `PORT` 與 `WORKSPACE_ROOT` 環境變數。
2. 建立共用的 `AppState`。
3. 載入 `~/.catdesk/config.toml`。
4. 啟用終端機 Raw Mode 與 Alternate Screen。
5. 顯示啟動畫面與 Binagotchy。
6. 讓使用者選擇 `Control Computer`、`Control Browser` 或 `Both`。
7. 需要時掃描並選擇 Chromium 瀏覽器。
8. 啟動本機 Axum Server。
9. 啟動 ngrok Tunnel。
10. 進入主要 TUI，持續顯示連線、工具呼叫、日誌與 Token 統計。

主要入口是 `#[tokio::main] async fn main()`；因此程式從一開始就運行在 Tokio 非同步 Runtime 上。

## MCP Server

MCP 是 Model Context Protocol。它是一套標準格式，讓 ChatGPT 知道有哪些工具、工具參數是什麼，以及如何取得執行結果。

CatDesk 在 [`src/mcp.rs`](../src/mcp.rs) 自行處理 JSON-RPC，主要支援：

- `server/discover`
- `tools/list`
- `tools/call`
- `resources/list`
- `resources/read`
- `ping`

目前使用的 MCP protocol version 是 `2026-07-28`。

HTTP 層在 [`src/server.rs`](../src/server.rs) 驗證：

- `MCP-Protocol-Version`
- `Mcp-Method`
- `Mcp-Name`
- JSON 內的 `_meta`
- Client capabilities
- protocol version 是否一致

因此它不是收到任意 HTTP 請求就直接執行命令，而是先確認請求符合 MCP 格式。

## 工具模式

設定模型在 [`src/state.rs`](../src/state.rs) 的 `Mode` 與 `ToolMode`。

### Computer、Browser、Both

- `Computer`：啟用 Workspace 與 Shell 工具。
- `Browser`：啟用瀏覽器 DevTools 工具。
- `Both`：兩者都啟用。

### MultiTools、ReadOnly

`MultiTools` 提供完整工具：

- `catdesk_instruction`
- `read`
- `search`
- `write`
- `edit`
- `delete`
- `run_command`
- `start_command`
- `poll_command`
- `cancel_command`

`ReadOnly` 只提供安全的讀取與搜尋功能，不提供寫檔、刪除或 Shell 執行。

CatDesk 會強制 ChatGPT 先成功呼叫 `catdesk_instruction`，取得操作規則後才允許使用其他 CatDesk 工具。

## Workspace 與檔案工具

檔案工具集中在 [`src/workspace_tools.rs`](../src/workspace_tools.rs)。

### `read`

一次讀取多個文字檔案，並限制：

- 最多 32 個檔案
- 整批最多 512 KB
- 單檔有大小上限
- 回報檔案大小、行數與是否截斷

### `search`

搜尋順序是：

1. 有 `rg` 就使用 ripgrep。
2. 沒有就使用 `grep`。
3. 再不行就使用內建 Rust 搜尋。

支援 Regex、固定字串、忽略大小寫、Glob、前後文、隱藏檔案與 ignore 規則。

### `write`、`edit`、`delete`

- `write`：建立或覆寫檔案，可選擇建立父目錄。
- `edit`：用精確文字或指定行號範圍修改檔案。
- `delete`：刪除檔案或目錄。

`edit` 會先在記憶體中完成整批編輯，全部成功後才寫回檔案；其中一個操作失敗時，不會留下半完成的結果。

所有路徑都會先 canonicalize，再檢查是否仍位於 Workspace 內，因此 `../../other-file` 這類路徑會被拒絕。相關檢查在 [`src/command.rs`](../src/command.rs)。

## Shell 與背景工作

短命令使用 `run_command`；編譯、長時間測試或開發伺服器使用：

```text
start_command  → 立即取得 job_id
poll_command   → 取得增量輸出
cancel_command → 停止工作
```

[`src/command_jobs.rs`](../src/command_jobs.rs) 提供：

- UUID 工作 ID
- `running`、`succeeded`、`failed`、`cancelled`、`timed_out` 狀態
- 增量輸出 cursor
- Timeout 與 Cancellation
- 同時最多 8 個工作
- 輸出大小上限
- idempotency，避免相同請求重複建立工作

不同平台的 Shell：

- Windows：PowerShell
- macOS：`/bin/bash`
- Linux：透過 Landlock helper 啟動 Bash

CatDesk 會為命令建立獨立的程序群組。Unix 使用 process group，Windows 使用 Job Object，所以取消命令時通常會連同編譯器、測試程式等子程序一起清掉。實作在 [`src/process_runner.rs`](../src/process_runner.rs)。

## Shell 指令攔截

CatDesk 使用 `tree-sitter-bash` 解析 Shell 指令。對於簡單的：

- `find`
- `tree`
- `ls -R`
- `rg --files`
- `mv`

它可能不真的啟動 Shell，而是改用自己的 Workspace API，回傳一致的結構化結果。這樣更容易套用 Workspace 邊界，也方便 Widget 顯示資料。

## 檔案變更追蹤與 Diff

長時間命令可以在開始前先拍攝 Workspace snapshot，之後再拍一次，找出新增、刪除與修改的檔案。

```text
開始工作
  ↓
記錄檔案狀態
  ↓
執行命令
  ↓
再次記錄檔案狀態
  ↓
產生 unified diff
```

實作在 [`src/change_tracking`](../src/change_tracking)。它會避開 `.git` 等版本控制內部檔案，尊重 `.gitignore`，並限制追蹤檔案數與 Diff 大小。

## Linux Landlock 安全機制

Linux 使用 Landlock ABI v3 限制命令存取檔案系統。一般情況下只允許 Workspace 與特定暫存目錄寫入，系統必要路徑則只讀。

若核心不支援 Landlock，CatDesk 預設拒絕執行未隔離的命令。只有明確設定以下環境變數，才允許不使用核心沙盒：

```text
CATDESK_ALLOW_UNSANDBOXED_LINUX=1
```

相關實作在 [`src/linux_sandbox.rs`](../src/linux_sandbox.rs)。

## 瀏覽器控制

瀏覽器功能由外部的 `chrome-devtools-mcp` 提供。CatDesk 會執行：

```text
npx -y chrome-devtools-mcp@latest
```

然後透過 stdin/stdout 傳送 JSON-RPC。啟動瀏覽器模式時，CatDesk 會掃描 Chrome、Chromium、Edge、Brave、Vivaldi 與 Opera，必要時使用 Remote Debugging port 啟動獨立瀏覽器程序。

Firefox 目前可以被辨識，但因為尚未接好 Firefox 的 CDP bridge，所以標記為不支援。

相關程式在 [`src/browser.rs`](../src/browser.rs) 與 [`src/devtools.rs`](../src/devtools.rs)。

## ngrok 網路層

本機 Server 預設只監聽：

```text
127.0.0.1:3200
```

[`src/ngrok.rs`](../src/ngrok.rs) 使用 ngrok Rust SDK：

1. 讀取 ngrok authtoken。
2. 建立 ngrok session。
3. 將公開流量轉送到本機 port。
4. 取得公開 URL。
5. 組成 `https://domain/random-path/mcp`。

MCP random path 與 ngrok 設定會保存到 `~/.catdesk/config.toml`，讓 Connector 可以重複使用。

這個 URL 沒有額外登入驗證，因此它本身就像一把鑰匙。知道 URL 的人可能直接操作你的電腦，不能把它分享給其他人。

## ChatGPT Widget

Widget 是 [`src/widget/catdesk_dashboard.html`](../src/widget/catdesk_dashboard.html)，使用原生 HTML、CSS 與 JavaScript，沒有 React 或 Vue。

MCP Server 透過 `resources/list` 與 `resources/read` 提供：

```text
ui://widget/catdesk-dashboard.html
```

每次工具回應也會附上 `catdesk/widgetPayload` metadata。Widget 使用 `window.openai.toolResponseMetadata`、`postMessage` 與 `openai:set_globals` 取得資料，顯示：

- 命令狀態
- stdout/stderr
- 修改檔案
- unified diff
- Token 統計
- 錯誤訊息
- Binagotchy 動畫

## Token 統計

CatDesk 不會取得 ChatGPT 官方 Token 數字，而是用 `tiktoken-rs` 的 `o200k_base` 對工具輸入與輸出做估算。

它不計算：

- 完整 ChatGPT 對話
- 隱藏 System Prompt
- Reasoning tokens
- OpenAI 內部處理量

所以 UI 的數字是工具資料的估算值，不是官方帳單數字。

## 共用狀態與設定

Server、TUI、ngrok 與背景工作共用：

```rust
Arc<Mutex<AppState>>
```

- `Arc` 讓多個非同步任務共用狀態。
- `Mutex` 保證同一時間只有一個任務修改狀態。
- channel 將 Server 事件傳給 TUI。

設定模型在 [`src/state.rs`](../src/state.rs)，主要保存：

- ngrok token/domain
- MCP random slug
- Computer/Browser/Both 模式
- MultiTools/ReadOnly 模式
- TUI theme
- Widget 顯示設定
- 選中的瀏覽器
- 使用量統計
- Binagotchy 設定

專案沒有資料庫，主要使用 TOML 設定檔與日誌檔。

## Binagotchy 圖像系統

Binagotchy 不是 AI 生成圖片，也沒有使用 diffusion model。它使用 Rust 的 `image`、`rand`、pixel mask、幾何繪圖、透明度合成與隨機種子，程式化產生：

- PNG
- GIF
- TUI 文字畫面
- Widget 像素動畫

相關程式在 [`src/binagotchy_gen`](../src/binagotchy_gen) 與 [`src/mascot.rs`](../src/mascot.rs)。

## npm 安裝方式

使用者安裝的是 npm 套件：

```bash
npm install -g catdesk
```

但 npm 套件主要是安裝器與啟動 wrapper。`npm/postinstall.js` 會：

1. 判斷作業系統與 CPU 架構。
2. 從 GitHub Release 下載對應 Rust binary。
3. 下載 `SHA256SUMS`。
4. 驗證 binary 雜湊。
5. 將 binary 放到 `npm/bin`。

執行 `catdesk` 時，`npm/catdesk.js` 再啟動真正的 Rust 執行檔，並保留目前目錄與環境變數。

目前的預編譯平台包括 Linux x64、Linux arm64、macOS x64、macOS arm64 與 Windows x64。

## 實際請求範例

假設使用者請 ChatGPT：

> 把 `src/config.rs` 的 timeout 改成 30 秒，然後執行測試。

大致流程是：

1. ChatGPT 呼叫 `catdesk_instruction`。
2. ChatGPT 呼叫 `read` 讀取檔案。
3. CatDesk 驗證 MCP header 與 Workspace 路徑。
4. CatDesk 回傳檔案內容。
5. ChatGPT 呼叫 `edit`。
6. CatDesk 驗證 old text 是否仍然吻合。
7. CatDesk 寫回檔案。
8. ChatGPT 呼叫 `start_command` 執行測試。
9. CatDesk 建立背景 Job。
10. ChatGPT 使用 `poll_command` 取得測試輸出。
11. CatDesk 比對修改前後的檔案狀態。
12. Widget 顯示測試結果與檔案 Diff。

## 主要模組對照

| 檔案或目錄 | 職責 |
|---|---|
| `src/main.rs` | 程式入口、啟動流程、TUI 主迴圈 |
| `src/server.rs` | Axum 路由、HTTP、MCP 請求驗證 |
| `src/mcp.rs` | JSON-RPC、工具清單、工具呼叫與 Widget metadata |
| `src/state.rs` | 共用狀態、模式、設定與使用量 |
| `src/workspace_tools.rs` | 讀寫搜尋刪除與檔案編輯 |
| `src/command.rs` | 路徑檢查、Shell 解析與命令攔截 |
| `src/process_runner.rs` | 跨平台程序啟動、輸出與程序樹清理 |
| `src/command_jobs.rs` | 長時間命令的背景工作管理 |
| `src/change_tracking/` | Snapshot、ignore 與 Diff |
| `src/browser.rs` | 瀏覽器偵測與 Remote Debugging |
| `src/devtools.rs` | chrome-devtools-mcp JSON-RPC bridge |
| `src/ngrok.rs` | ngrok Tunnel |
| `src/widget/` | ChatGPT 內嵌 Widget |
| `src/mascot.rs`、`src/binagotchy_gen/` | Binagotchy 圖像與動畫 |

## 總結

CatDesk 的技術本質是：

> ChatGPT Web 的大腦
> + CatDesk 的本機手腳
> + MCP 的共同語言
> + ngrok 的網路通道
> + Rust 的跨平台執行與安全控制

