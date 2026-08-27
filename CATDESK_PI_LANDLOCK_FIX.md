# CatDesk Raspberry Pi Landlock 修正與部署

## 問題背景

目前 Raspberry Pi 環境：

```text
Linux raspberrypi 6.12.96+rpt-rpi-2712 aarch64
Debian GNU/Linux 12 (bookworm)
# CONFIG_SECURITY_LANDLOCK is not set
```

CatDesk 原本在 `src/linux_sandbox.rs` 使用 Landlock，並要求：

```rust
CompatLevel::HardRequirement
```

且要求：

```rust
RulesetStatus::FullyEnforced
```

由於 Raspberry Pi 官方 kernel 沒有編譯 `CONFIG_SECURITY_LANDLOCK`，任何 `run_command` / `start_command` 都會在 shell 啟動前失敗，典型錯誤：

```text
HandleAccesses(Fs(Compat(Access(Incompatible { ... }))))
```

本 fork 已修改為：

- Landlock 正常可用時：繼續使用完整 Landlock sandbox。
- Landlock 部分生效時：仍拒絕執行，避免誤以為 sandbox 完整。
- Landlock 完全不可用時：只有設定 `CATDESK_ALLOW_UNSANDBOXED_LINUX=1` 才允許 command 執行。
- 沒有設定 opt-in 時仍維持 fail-safe 行為。
- 已加入對應 regression tests。

> 注意：在 `CATDESK_ALLOW_UNSANDBOXED_LINUX=1` 模式下，CatDesk 的 `read/write/edit/delete` 仍有 Workspace 路徑檢查，但 shell command 不再有 Landlock kernel filesystem confinement。請只在你信任的 Raspberry Pi / VM / container 中使用。

---

# 1. 進入 fork

```bash
cd ~/github/CatDesk
```

確認 remote：

```bash
git remote -v
```

預期 `origin` 指向：

```text
https://github.com/jamesliu69/CatDesk.git
```

查看目前修改：

```bash
git status --short
```

應至少看到：

```text
M src/linux_sandbox.rs
?? scripts/deploy-pi-local.sh
?? CATDESK_PI_LANDLOCK_FIX.md
```

---

# 2. 確認 Rust 環境

```bash
rustc --version
cargo --version
```

如果找不到 Rust：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

然後重新確認：

```bash
rustc --version
cargo --version
```

---

# 3. 執行測試

先跑完整測試：

```bash
cd ~/github/CatDesk
cargo test
```

必須確認最後沒有：

```text
FAILED
```

且 exit code 為 0：

```bash
echo $?
```

應顯示：

```text
0
```

---

# 4. Release 編譯

```bash
cargo build --release
```

確認 binary：

```bash
ls -lh target/release/catdesk
file target/release/catdesk
```

預期為 ARM64 / aarch64 Linux executable。

---

# 5. 找出目前 npm 安裝的 CatDesk

```bash
command -v catdesk
npm root -g
```

CatDesk npm launcher 通常位於：

```text
$(npm root -g)/catdesk/npm/catdesk.js
```

實際 binary 通常位於：

```text
$(npm root -g)/catdesk/npm/bin/catdesk
```

確認：

```bash
CATDESK_NPM_ROOT="$(npm root -g)/catdesk/npm"
ls -lh "$CATDESK_NPM_ROOT/bin/catdesk"
```

---

# 6. 備份目前 CatDesk binary

```bash
CATDESK_NPM_ROOT="$(npm root -g)/catdesk/npm"
cp "$CATDESK_NPM_ROOT/bin/catdesk" \
   "$CATDESK_NPM_ROOT/bin/catdesk.backup-$(date +%Y%m%d-%H%M%S)"
```

確認備份：

```bash
ls -lh "$CATDESK_NPM_ROOT/bin/"
```

---

# 7. 部署新 binary

```bash
CATDESK_NPM_ROOT="$(npm root -g)/catdesk/npm"
cp ~/github/CatDesk/target/release/catdesk \
   "$CATDESK_NPM_ROOT/bin/catdesk"
chmod +x "$CATDESK_NPM_ROOT/bin/catdesk"
```

確認：

```bash
ls -lh "$CATDESK_NPM_ROOT/bin/catdesk"
```

---

# 8. 建立 Raspberry Pi 專用 launcher

建立 `~/.local/bin`：

```bash
mkdir -p ~/.local/bin
```

建立 launcher：

```bash
cat > ~/.local/bin/catdesk-pi <<'EOF'
#!/usr/bin/env bash
set -e

export CATDESK_ALLOW_UNSANDBOXED_LINUX=1
exec catdesk "$@"
EOF

chmod +x ~/.local/bin/catdesk-pi
```

確認：

```bash
cat ~/.local/bin/catdesk-pi
```

---

# 9. 啟動 AutoScwaw Workspace

先停止目前舊的 CatDesk instance。

如果 CatDesk 正在前景終端執行，按：

```text
Ctrl+C
```

然後：

```bash
cd ~/github/AutoScwaw
WORKSPACE_ROOT="$PWD" ~/.local/bin/catdesk-pi
```

如果你希望之後直接輸入 `catdesk-pi`，確認 `~/.local/bin` 在 PATH：

```bash
echo "$PATH"
```

若沒有，可加入：

```bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

---

# 10. 驗證 CatDesk command 功能

重新連接 ChatGPT 的 CatDesk2 connector 後，要求執行：

```bash
pwd
```

預期：

```text
/home/pi/github/AutoScwaw
```

接著測：

```bash
git status --short --branch
```

再測：

```bash
uname -a
```

如果這些都能正常輸出，就代表原本的 Landlock `Access(Incompatible ...)` 問題已排除。

---

# 11. 額外安全驗證

由於 Pi 沒有 Landlock，shell command 在 opt-in fallback 模式下沒有 kernel filesystem sandbox。

確認 Workspace 的檔案工具仍拒絕 workspace 外路徑；不要使用 CatDesk 對不信任的 prompt 開放這台主機。

建議：

- CatDesk 使用專用 Linux 帳號。
- 不要使用 root 啟動 CatDesk。
- 不要把 MCP/ngrok URL 分享給其他人。
- 重要主機建議使用 container / VM。

---

# 12. Commit 到自己的 fork（可選）

先看最近 commit 格式：

```bash
git log --oneline -n 5
```

確認修改：

```bash
git diff -- src/linux_sandbox.rs scripts/deploy-pi-local.sh CATDESK_PI_LANDLOCK_FIX.md
```

加入：

```bash
git add src/linux_sandbox.rs scripts/deploy-pi-local.sh CATDESK_PI_LANDLOCK_FIX.md
```

提交範例：

```bash
git commit -m "fix(linux): allow explicit fallback without Landlock"
```

推到你的 fork：

```bash
git push origin main
```

如果目前不是 `main`，先確認：

```bash
git branch --show-current
```

再把上面的 `main` 換成實際 branch。

---

# 13. 回滾方法

如果新版有問題，先停止 CatDesk。

找備份：

```bash
CATDESK_NPM_ROOT="$(npm root -g)/catdesk/npm"
ls -lt "$CATDESK_NPM_ROOT/bin/catdesk.backup-"*
```

選最新備份，例如：

```text
catdesk.backup-20260828-070000
```

回復：

```bash
cp "$CATDESK_NPM_ROOT/bin/catdesk.backup-20260828-070000" \
   "$CATDESK_NPM_ROOT/bin/catdesk"
chmod +x "$CATDESK_NPM_ROOT/bin/catdesk"
```

再用原方式啟動：

```bash
cd ~/github/AutoScwaw
WORKSPACE_ROOT="$PWD" catdesk
```

---

# 最短部署流程

如果前面檢查都沒問題，可以直接執行 repo 內已建立的部署腳本：

```bash
cd ~/github/CatDesk
bash scripts/deploy-pi-local.sh
```

然後：

```bash
cd ~/github/AutoScwaw
WORKSPACE_ROOT="$PWD" ~/.local/bin/catdesk-pi
```

重新連接 CatDesk2 後，用：

```bash
git status --short --branch
```

作為最終驗證。
