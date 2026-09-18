# CatDesk v0.8.0 選擇性整合紀錄

日期：2026-09-12  
整合分支：`integrate/upstream-v0.8.0-20260912`  
Fork 起點：`c95cfd4`  
上游來源：`Xeift/CatDesk`，`41900ba`（`v0.8.0`）

## 整合方式與範圍

採用功能層級移植，而非整支 `upstream/main` merge 或覆蓋整個檔案。保留 fork 的外部 HTTPS tunnel、非同步啟動、DevTools 修正、指令／設定快取、批次讀取與背景命令變更追蹤優化。

| 項目 | 實作 |
| --- | --- |
| Change Tracking | 移植 canonical workspace 與 symlink 路徑正規化，回報 workspace-relative 路徑；保留既有公開常數與變更回報介面。 |
| TUI panic | TUI 所在執行緒發生 panic 時還原 raw mode、bracketed paste、mouse capture、alternate screen。 |
| Session Handoff | 新增 `src/handoff.rs`、`create_handoff`、MCP input/output schema、ReadOnly 支援及 Widget 摘要。 |
| Handoff 指引 | 專案識別資訊只加入 workspace-specific instruction；保留既有 connect-time guidance 與 cache 分層，不讓基底指引依賴特定工作區。 |
| 繁體中文 | 補齊 Settings、Dashboard、Browser selection、Connector refresh、MCP bootstrap、提示訊息及常見 Log；中文字寬、截斷、換行及對齊依 terminal cells 計算。 |
| Linux Sandbox | 優先使用工作區外可信任的 `bwrap`；補上 executable／symlink 驗證、user/PID/IPC/UTS namespaces、SSH metadata allowlist 及 SSH Agent socket 轉送。 |
| Sandbox fallback | 僅在沒有可信任 `bwrap` 時使用原有 Landlock；限制無法完整套用時拒絕執行，沒有無隔離的 fallback。 |
| 文件與測試 | 更新中英文 README、工具數量及使用限制；新增真實 Release binary 的本機端對端測試 `tests/runtime_smoke.py`。 |

## 驗證時額外發現並修正的問題

原有 `process_runner` 的單元測試在 `cfg(test)` 下使用普通 shell，並不走真正的 Linux Sandbox。真實 Release binary 的 MCP 測試發現：逾時後，Bubblewrap 的 namespace 內子程序仍可能繼續寫入檔案。

直接移除 `--die-with-parent` 可以避免任務被暫時性的 Tokio 工作執行緒回收誤殺，卻不能保證 namespace 內子程序跟著命令管理程序結束。直接把這個參數加回去也不足以同時滿足兩個條件。

目前改為透過持續存在的父程序啟動並等待 Bubblewrap，再搭配 `--die-with-parent` 連動 namespace 清理。父程序使用固定腳本及逐一傳遞的參數，不將使用者輸入串接成 host-side shell 指令；`-p` 禁止 host-side 父程序載入匯入函式及 shell 啟動設定。既有 `ProcessTreeGuard` 與 `process_runner.rs` 保持不變。

新增 `bubblewrap_survives_spawning_thread_but_not_its_command_owner` 回歸測試：先讓產生子程序的執行緒結束，確認命令仍存活，再終止命令管理程序，確認後續寫入不會發生。測試已先重現失敗，再驗證修正。

Handoff Widget 另補上可見的檔名與摘要，明確顯示檔案尚未存入 Library，避免將「產生內容」誤認為「已持久保存」。

## 保留與未納入項目

`src/process_runner.rs`、`src/devtools.rs`、`src/command_jobs.rs`、`src/workspace_tools.rs`、`.github/workflows/release.yml` 與 `package.json` 保持 fork 原內容。`run_app`、`run_headless`、`start_services`、預設繁中及五秒自動選擇 Computer 的流程也保留。

沒有重新加入內建 ngrok、ngrok 設定畫面、npm 自動發佈或強制 Bubblewrap-only 架構。此輪未採用可選的 rustls／musl 發行調整，也未變更套件版本號；這是上游功能的選擇性整合，不是上游原版 v0.8.0 發行包。

## 使用與安全限制

Handoff 只回傳檔名與 Markdown；仍需 ChatGPT 使用 Library 工具保存。專案識別來自啟動時的 `WORKSPACE_ROOT`，不是每個命令傳入的 `cwd`。Handoff 不會備份未提交檔案、不會延續執行中的 Job，也不應包含憑證或機密。Library 的實際雲端保存／恢復不屬於本次本機測試。

SSH 只加入一般檔案形式的 config、known_hosts、known_hosts2 與公鑰，不加入 SSH 私鑰、公鑰符號連結或 `.pub` 目錄。保留 `.git-credentials` 與 `.config/gh/hosts.yml` 的指定唯讀權限。唯讀憑證與 SSH Agent 仍屬敏感權限；Sandbox 並不代表可以把正式憑證交給不受信任的程式。

目前 Linux 執行環境沒有可用的 Landlock，已驗證其拒絕執行行為；沒有宣稱通過 Landlock 正常執行測試。沒有修改 kernel、LSM 或系統安全設定。Windows、macOS 的原生建置／互動式操作亦未在此 Linux 環境執行。

## 驗證結果與重現方式

| 驗證 | 結果 |
| --- | --- |
| `cargo fmt --check` | 通過。 |
| `git diff --check` | 通過。 |
| `cargo test --locked --all-targets` | 279 passed、0 failed。 |
| `cargo test --locked --release` | 279 passed、0 failed。 |
| `cargo build --locked --release` | 通過，產生 `target/release/catdesk`。 |
| `python3 tests/runtime_smoke.py --landlock-unavailable` | Bubblewrap 端對端與 Landlock 不可用時的拒絕執行檢查通過。 |

端對端測試使用獨立的 HOME、工作區、暫存檔、合成 SSH Agent socket 與 loopback port，測試 MCP discovery、11 個工具註冊、Handoff 不寫入工作區、檔案隔離、指定 HTTPS 憑證存取、SSH Agent 連線、長時間 Job、incremental polling、exit status、取消、逾時及正常結束後的子程序清理。測試不連線外部 Git／SSH 服務，也不操作現有 CatDesk 服務。

可用 Landlock 的環境可執行 `python3 tests/runtime_smoke.py --landlock`，以要求正常執行測試；`--landlock-unavailable` 明確測試相反情境，不會把不支援的環境當成正常執行通過。

本次沒有 commit、push、建立上游 PR、部署或重新啟動現有服務。整合後的執行檔不會自動取代目前運行版本。
