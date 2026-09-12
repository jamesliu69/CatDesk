#!/usr/bin/env python3
"""Exercise a built CatDesk on loopback with disposable files, never the live service.

Run from the repository root: python3 tests/runtime_smoke.py
The test needs Linux, bubblewrap, git and Python 3.10 or later.
"""
from __future__ import annotations

import argparse
import itertools
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from typing import Any


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def exercise(binary: Path, parent: Path, use_landlock: bool, expect_unavailable: bool = False) -> list[str]:
    checks: list[str] = []
    case = parent / ("landlock" if use_landlock else "bubblewrap")
    workspace, home = case / "workspace", case / "home"
    for directory in (workspace, home / ".catdesk", home / ".ssh", case / "tmp"):
        directory.mkdir(parents=True)
    (home / ".ssh/id_ed25519").write_text("synthetic private fixture\n")
    (home / ".ssh/id_ed25519.pub").write_text("synthetic public fixture\n")
    (home / ".git-credentials").write_text("synthetic HTTPS fixture\n")
    outside = case / "outside.txt"
    outside.write_text("outside workspace fixture\n")
    subprocess.run(["git", "-C", str(workspace), "init", "-q"], check=True, timeout=10)
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    slug = "isolated-integration-smoke"
    (home / ".catdesk/config.toml").write_text(
        'theme = "concise"\nmode = "computer"\ntoolMode = "multiTools"\n'
        'showDetailMode = "disable"\npublicBaseUrl = "https://integration.invalid"\n'
        f'mcpSlug = "{slug}"\nchatgptConnectorRevision = 3\n', encoding="utf-8"
    )
    environment = os.environ.copy()
    environment.update(HOME=str(home), WORKSPACE_ROOT=str(workspace), PORT=str(port), TMPDIR=str(case / "tmp"))
    environment.pop("SSH_AUTH_SOCK", None)
    if use_landlock:
        # No bwrap in this PATH; absolute /bin/bash is still used by the real helper.
        binaries = case / "bin"
        binaries.mkdir()
        for name in ("git", "python3", "sleep"):
            source = shutil.which(name)
            require(source is not None, f"Missing prerequisite: {name}")
            (binaries / name).symlink_to(source)
        environment["PATH"] = str(binaries)
    else:
        environment["PATH"] = "/usr/bin:/bin"
    agent = socket.socket(socket.AF_UNIX)
    agent_path = case / "agent.sock"
    agent.bind(str(agent_path))
    agent.listen(1)
    agent.settimeout(5)
    environment["SSH_AUTH_SOCK"] = str(agent_path)
    ids = itertools.count(1)
    endpoint = f"http://127.0.0.1:{port}/{slug}/mcp"
    client = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def rpc(method: str, params: dict[str, Any] | None = None, allow_error: bool = False) -> dict[str, Any]:
        params = dict(params or {})
        params["_meta"] = {
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {},
            "io.modelcontextprotocol/clientInfo": {"name": "catdesk-runtime-smoke", "version": "1.0.0"},
        }
        headers = {"Content-Type": "application/json", "mcp-protocol-version": "2026-07-28", "mcp-method": method}
        if method == "tools/call":
            headers["mcp-name"] = params["name"]
        request = urllib.request.Request(endpoint, data=json.dumps({"jsonrpc": "2.0", "id": next(ids), "method": method, "params": params}).encode(), headers=headers)
        try:
            with client.open(request, timeout=15) as response:
                data = json.load(response)
        except urllib.error.HTTPError as error:
            body = error.read(8192).decode("utf-8", errors="replace")
            raise RuntimeError(f"HTTP {error.code} during {method}: {body}") from error
        require("error" not in data, f"JSON-RPC error: {data.get('error')}")
        result = data["result"]
        require(allow_error or not result.get("isError"), f"Tool failed: {result}")
        return result

    def tool(name: str, arguments: dict[str, Any], allow_error: bool = False) -> dict[str, Any]:
        return rpc("tools/call", {"name": name, "arguments": arguments}, allow_error)["structuredContent"]

    log_path = case / "server.log"
    with log_path.open("wb") as log:
        process = subprocess.Popen([str(binary), "--headless"], cwd=workspace, env=environment, stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True)
        try:
            deadline = time.monotonic() + 45
            while True:
                require(process.poll() is None, "Isolated server exited: " + log_path.read_text(errors="replace"))
                try:
                    discovered = rpc("server/discover")
                    break
                except (urllib.error.URLError, TimeoutError):
                    require(time.monotonic() < deadline, "Isolated server startup timed out: " + log_path.read_text(errors="replace"))
                    time.sleep(0.1)
            require("create_handoff" in discovered["instructions"], "Discovery omitted handoff guidance")
            tool("catdesk_instruction", {})
            descriptors = rpc("tools/list")["tools"]
            require(len(descriptors) == 11, f"Unexpected tool count: {len(descriptors)}")
            require(any(item["name"] == "create_handoff" for item in descriptors), "Handoff tool missing")
            checks.append("MCP discovery and 11-tool registration")
            before = sorted(str(p.relative_to(workspace)) for p in workspace.rglob("*"))
            handoff = tool("create_handoff", {"goal": "Verify isolated runtime integration", "validation": ["loopback smoke test"]})
            require(handoff["filename"].startswith("catdesk_handoff_"), "Invalid handoff identity")
            require("Verify isolated runtime integration" in handoff["content"], "Handoff content missing")
            require(before == sorted(str(p.relative_to(workspace)) for p in workspace.rglob("*")), "Handoff changed the workspace")
            checks.append("Handoff prepares context without writing workspace")
            if expect_unavailable:
                result = tool("run_command", {"command": "printf unconfined > refused.txt", "timeout": 10000}, allow_error=True)
                require(not result["success"], "Unavailable Landlock must reject command execution")
                require("Landlock sandbox is unavailable" in result["stderr"], "Expected the explicit unavailable-Landlock error")
                require(not (workspace / "refused.txt").exists(), "Command escaped the unavailable sandbox")
                checks.append("Unavailable Landlock rejects execution without unconfined fallback")
                return checks
            command = (
                f"test ! -r {shlex.quote(str(outside))} && "
                'test ! -r "$HOME/.ssh/id_ed25519" && '
                'test -r "$HOME/.git-credentials" && '
                "printf checked > result.txt && git --version && printf sandbox-ok"
            )
            execution = tool("run_command", {"command": command, "timeout": 10000})
            require(execution["success"] and "sandbox-ok" in execution["stdout"], f"Sandbox command failed: {execution}")
            require((workspace / "result.txt").read_text() == "checked", "Workspace not writable")
            checks.append("Filesystem confinement, workspace writes and HTTPS credential allowlist")
            failure = tool("run_command", {"command": "exit 7", "timeout": 10000}, allow_error=True)
            require(failure["exitCode"] == 7 and not failure["success"], "Sandbox lost the command exit status")
            if not use_landlock:
                code = "import os,socket; s=socket.socket(socket.AF_UNIX); s.connect(os.environ['SSH_AUTH_SOCK']); s.sendall(b'catdesk-test'); s.close(); print('agent-ok')"
                result = tool("run_command", {"command": "python3 -c " + shlex.quote(code), "timeout": 10000})
                require("agent-ok" in result["stdout"], "SSH agent is not reachable through sandbox")
                connection, _ = agent.accept()
                with connection:
                    require(connection.recv(64) == b"catdesk-test", "Agent socket did not receive test message")
                checks.append("Synthetic SSH Agent socket connection through real Bubblewrap")
            job = tool("start_command", {"command": "sleep 12; printf background-ok", "timeout": 30000})
            cursor, output = 0, ""
            deadline = time.monotonic() + 35
            while True:
                snapshot = tool("poll_command", {"job_id": job["jobId"], "after": cursor, "wait_ms": 1000})
                output += "".join(event["text"] for event in snapshot["events"])
                cursor = snapshot["nextCursor"]
                if snapshot["state"] != "running" and not snapshot["hasMoreOutput"]:
                    require(snapshot["state"] == "succeeded", f"Background command failed: {snapshot}")
                    break
                require(time.monotonic() < deadline, "Background job did not finish")
            require(output == "background-ok", f"Lost or duplicated job output: {output!r}")
            checks.append("Long-running job lifetime and incremental polling")
            job = tool("start_command", {"command": "sleep 1; printf survived > cancelled.txt", "timeout": 10000})
            cancelled = tool("cancel_command", {"job_id": job["jobId"]})
            require(cancelled["state"] == "cancelled", f"Cancellation failed: {cancelled}")
            timed_out = tool("run_command", {"command": "sleep 1; printf survived > timed-out.txt", "timeout": 100}, allow_error=True)
            require(timed_out["timedOut"], "Command did not time out")
            time.sleep(1.2)
            require(not (workspace / "cancelled.txt").exists(), "Cancelled job retained a live child")
            require(not (workspace / "timed-out.txt").exists(), "Timed-out command retained a live child")
            detached = tool("run_command", {"command": "(sleep 1; printf survived > detached.txt) & printf root-done", "timeout": 10000})
            require(detached["success"] and "root-done" in detached["stdout"], "Root completion was not reported")
            time.sleep(1.2)
            require(not (workspace / "detached.txt").exists(), "Completed command left a detached child")
            checks.append("Exit status, cancellation, timeout and successful-root child cleanup")
            return checks
        finally:
            agent.close()
            if process.poll() is None:
                process.send_signal(signal.SIGINT)
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=5)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/catdesk"))
    landlock = parser.add_mutually_exclusive_group()
    landlock.add_argument("--landlock", action="store_true", help="Also exercise the retained Landlock backend; requires kernel support")
    landlock.add_argument("--landlock-unavailable", action="store_true", help="Assert that an unavailable Landlock backend refuses execution; not a positive Landlock execution test")
    arguments = parser.parse_args()
    require(sys.platform.startswith("linux"), "This runtime smoke test requires Linux")
    binary = arguments.binary.resolve(strict=True)
    require(shutil.which("bwrap") is not None, "bubblewrap must be installed")
    root = Path(".logs").resolve()
    root.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="smoke-", dir=root) as temporary:
        checks = exercise(binary, Path(temporary), False)
        for item in checks:
            print("PASS Bubblewrap: " + item)
        if arguments.landlock or arguments.landlock_unavailable:
            for item in exercise(binary, Path(temporary), True, arguments.landlock_unavailable):
                print("PASS Landlock: " + item)
    if arguments.landlock_unavailable:
        print("Landlock positive execution was not tested: this run explicitly verifies unavailable-backend refusal.")
    print("All requested isolated runtime smoke checks passed; test servers and fixtures removed.")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        raise SystemExit(1)
