# Windows + Cloudflare Tunnel 雙 CatDesk 設定

這份 runbook 讓同一個 ChatGPT 帳號同時使用兩台 CatDesk：

- `@pi`：Cloudflare Tunnel 連到 Raspberry Pi 5，操作 Pi 5 的 workspace。
- `@win`：另一個 Cloudflare Tunnel 連到 Windows，操作 Windows 的 workspace。

Cloudflare Tunnel 只是網路通道；真正讀寫檔案與執行指令的是 Tunnel 所在機器上的 CatDesk。不要把 Pi 5 的 Connector URL 當成 Windows Connector URL 使用。

## 架構

```text
ChatGPT @pi ──> pi.example.com ──> cloudflared (Pi 5) ──> 127.0.0.1:3200 ──> Pi workspace

ChatGPT @win ─> win.example.com ─> cloudflared (Windows) ─> 127.0.0.1:3200 ──> Windows workspace
```

建議使用不同的 Tunnel、hostname 和 ChatGPT Connector。兩台機器可以各自使用預設的 `3200` port，因為 port 是每台機器本地的 port。

## 需要先準備的資料

以下值請替換成你自己的值，不要把 token、`cert.pem` 或 tunnel credentials commit 到 Git：

```text
Windows tunnel name: catdesk-windows
Windows tunnel UUID: <WINDOWS_TUNNEL_UUID>
Windows public hostname: catdesk-win.example.com
Windows project: C:\Projects\my-project
```

需要 Cloudflare 帳號管理權限來建立 Tunnel、hostname route，或從 Dashboard 取得 Windows connector token。Codex MCP 可以檢查本機檔案與執行命令，但不能代替你登入 Cloudflare Dashboard。

## 1. 安裝 Windows CatDesk

### 使用本次建置的執行檔

把下列檔案複製到 Windows 固定位置，並命名為 `catdesk.exe`：

```text
D:\Repo\CatDesk\dist\catdesk.exe
```

例如放在：

```text
C:\Tools\CatDesk\catdesk.exe
```

把 `C:\Tools\CatDesk` 加入使用者 PATH。開啟新的 PowerShell 後驗證：

```powershell
Get-Command catdesk
catdesk
```

### 使用 npm 發佈版

如果對應版本已經發佈到 GitHub Release，也可以使用：

```powershell
npm install -g catdesk
```

需要 Node.js 18 或更新版本。npm 安裝器會依 Windows 架構下載對應的 release binary。

## 2. 在 Windows 建立 Cloudflare Tunnel

### 建議方式：Cloudflare Dashboard 管理 Tunnel

1. 在 Cloudflare Zero Trust 建立新的 Tunnel，名稱例如 `catdesk-windows`。
2. 為 Windows 安裝 `cloudflared`，並使用 Dashboard 顯示的 Windows connector install command。
3. 新增一個 Public Hostname：

   ```text
   Hostname: catdesk-win.example.com
   Service: http://127.0.0.1:3200
   ```

4. 先讓 CatDesk 在 Windows 的 `127.0.0.1:3200` 監聽，再啟動 `cloudflared` connector。

若使用 Dashboard 產生的 token，請只在 PowerShell 或 Windows service 設定中使用，絕對不要貼到 Git、Markdown 或 ChatGPT 對話。

### locally-managed Tunnel

若你和 Pi 5 一樣使用 locally-managed Tunnel，Windows 的 `%USERPROFILE%\.cloudflared\config.yml` 可使用以下形式。完整欄位定義請參考 Cloudflare 的 [configuration file 文件](https://developers.cloudflare.com/tunnel/features/locally-managed-tunnels/configuration/)：

```yaml
tunnel: <WINDOWS_TUNNEL_UUID>
credentials-file: C:\Users\<WINDOWS_USER>\.cloudflared\<WINDOWS_TUNNEL_UUID>.json

ingress:
  - hostname: catdesk-win.example.com
    service: http://127.0.0.1:3200
  - service: http_status:404
```

先驗證規則，再建立 DNS route 並啟動 Tunnel：

```powershell
cloudflared tunnel ingress validate
cloudflared tunnel route dns catdesk-windows catdesk-win.example.com
cloudflared tunnel run catdesk-windows
```

確認前景執行正常後，再依 Cloudflare 的 [Windows service 指南](https://developers.cloudflare.com/tunnel/features/locally-managed-tunnels/as-a-service/windows/) 設定開機自動啟動。若 `cloudflared` 以 Windows service 執行，請確認 service 使用的帳號能讀取 `config.yml` 與 credentials JSON。

## 3. 啟動 Windows CatDesk

每個 Windows 專案啟動一個 CatDesk，workspace root 用絕對路徑指定：

```powershell
$env:WORKSPACE_ROOT = "C:\Projects\my-project"
$env:PORT = "3200"
catdesk
```

也可以用目前目錄作為 workspace：

```powershell
Set-Location C:\Projects\my-project
catdesk
```

啟動時：

1. 選 `Control Computer`，或需要瀏覽器時選 `Both`。
2. 第一次詢問 Public Base URL 時輸入：

   ```text
   https://catdesk-win.example.com
   ```

   這裡輸入 hostname origin，不要輸入 Pi 的網址，也不要自行加 `/mcp`。
3. 從 CatDesk TUI 複製完整 MCP Server URL。完整 URL 包含隨機 secret path，請視為密碼保護。

`WORKSPACE_ROOT` 預設為啟動 CatDesk 的目前目錄。設定檔會保存於 Windows 使用者的 `~\.catdesk\config.toml`。

## 4. 建立 Windows ChatGPT Connector

保留現有的 Pi Connector，例如 `CatDesk Pi` 或 `@pi`，再建立第二個 Connector：

```text
Name: CatDesk Windows
MCP Server URL: （Windows CatDesk TUI 顯示的完整 URL）
Authentication: None
```

在 ChatGPT 中選擇：

- `@pi`：需要操作 Pi 5 時使用。
- `@win` 或 `CatDesk Windows`：需要操作 Windows 時使用。

不要把完整 MCP URL 分享給其他人。`Authentication: None` 時，random secret path 是 capability secret，不是登入驗證。

## 5. 切換 Windows 專案

同一時間只需要一個 Windows CatDesk 時，先按 `Ctrl+C` 停止，再以新的 workspace 啟動：

```powershell
$env:WORKSPACE_ROOT = "C:\Projects\another-project"
catdesk
```

如果要同時執行多個 Windows workspace，必須為每個 CatDesk 使用不同 port、hostname 和 Connector。例如第二個 instance：

```powershell
$env:WORKSPACE_ROOT = "C:\Projects\another-project"
$env:PORT = "3201"
catdesk
```

第二條 Tunnel 的 origin 也必須改成 `http://127.0.0.1:3201`。不要讓同一個 CatDesk process 同時指向多個 workspace。

## 6. 驗證清單

在 Windows 逐項確認：

```powershell
# CatDesk binary 能被 PATH 找到
Get-Command catdesk

# 本機 MCP port 有程序監聽
Test-NetConnection 127.0.0.1 -Port 3200

# Tunnel connector 狀態（Tunnel 名稱或 UUID 擇一）
cloudflared tunnel info catdesk-windows
```

接著在 ChatGPT 選 `CatDesk Windows`，先呼叫 `list_resources`，再呼叫 `catdesk_instruction`，最後要求它讀取 Windows workspace 中一個已知的小文字檔。確認回傳的是 Windows 專案內容，而不是 Pi 5 的內容。

## 常見問題

| 現象 | 原因與處理 |
| --- | --- |
| Cloudflare 回 `502` | Windows CatDesk 沒啟動、port 不一致，或 `cloudflared` 指向錯誤的 port。先檢查 `Test-NetConnection`。 |
| Cloudflare 回 `404` | hostname 沒命中 ingress，或 URL 不是 CatDesk TUI 顯示的完整 MCP URL。 |
| Connector 仍讀到 Pi 檔案 | ChatGPT 選到 `@pi`，或 Windows Connector 仍使用 Pi 的完整 MCP URL。 |
| `catdesk` 找不到 | 重新開啟 PowerShell 讓 PATH 生效，並確認 PATH 指向包含 `catdesk.exe` 的資料夾。 |
| 顯示錯誤的 Windows 專案 | 檢查 `$env:WORKSPACE_ROOT`；未設定時 CatDesk 使用啟動當下的目前目錄。 |
| `3200` 已被占用 | 改用 `PORT=3201`，並同步修改 Cloudflare origin service。 |
| Tunnel service 開機後失效 | 檢查 Windows service 使用的帳號、`config.yml` 路徑及 credentials JSON 讀取權限。 |

## 可直接交給 Codex MCP 的請求

把以下內容貼給能操作 Windows 主機的 Codex MCP。它會先檢查狀態，再執行不需要 Cloudflare 帳號權限的本機步驟；需要 Dashboard 或 token 的地方會停下來請你提供資料。

```text
請依照 docs/windows-cloudflare-catdesk.md 設定 Windows 端 CatDesk，目標 workspace 是 C:\Projects\<PROJECT>。

先只做檢查，不要把任何 token、cert.pem、credentials JSON 或完整 MCP secret URL 寫入 repository：
1. 確認 catdesk.exe 存在並可由 PATH 找到。
2. 確認 cloudflared 已安裝、目前 Tunnel/config.yml 是否存在，以及 127.0.0.1:3200 是否可用。
3. 確認 CatDesk 應使用 WORKSPACE_ROOT=C:\Projects\<PROJECT>。

若缺少 Cloudflare Tunnel、Public Hostname 或 connector token，列出需要我在 Cloudflare Dashboard 手動完成的項目，不要猜測或建立新的 DNS/Tunnel。若本機設定已完整，啟動 cloudflared 與 CatDesk，並告訴我 ChatGPT Connector 應使用哪一個完整 MCP URL。完成後用一個安全的已知文字檔驗證 workspace 不是 Pi 5。
```
