#!/usr/bin/env python3
"""阶段0 接缝 0.6/0.7 探针：用 stdio JSON-RPC 连 `dsh --profile acp`。

2026-10-24 审计指出原实现有竞态：所有 stdio 消息共用一个 queue，
`session/prompt` 的响应会被 collect 线程抢走，导致通知丢失、agent_text 为空。
修法：按 `id` 分流的响应队列 + 独立的通知队列。

用法：python3 tools/probe_acp.py [--prompt "自定义任务"]
输出：JSON 到 stdout（供 evidence 存档），关键字段：
  - session_id           建会话成功与否
  - agent_text           助手流式文本拼起来的结果
  - stop_reason          end_turn / 其他
  - notifications        收到的通知条数
"""
import json
import subprocess
import sys
import threading
import time
import queue

DEFAULT_PROMPT = (
    "请执行一个很小的任务：运行 `ls /home/wings/Code_Project/diemeng`，"
    "然后用一句话告诉我目录里有哪些文件。不要修改任何文件。"
)


class Conn:
    """NDJSON JSON-RPC 连接：响应和通知分开排队（修复审计指出的竞态）。"""

    def __init__(self, cmd):
        self.p = subprocess.Popen(
            cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        self.responses: dict[int, "queue.Queue"] = {}
        self.notifications: "queue.Queue" = queue.Queue()
        self.stderr_lines: list[str] = []
        self.next_id = 1
        self._lock = threading.Lock()
        threading.Thread(target=self._read_stdout, daemon=True).start()
        threading.Thread(target=self._read_stderr, daemon=True).start()

    def _read_stdout(self):
        for line in self.p.stdout:
            line = line.strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                self.notifications.put({"__raw__": line})
                continue
            if "id" in msg and ("result" in msg or "error" in msg):
                # 这是响应：投递到该 id 专属队列
                q = self.responses.get(msg["id"])
                if q is not None:
                    q.put(msg)
                else:
                    self.notifications.put(msg)
            else:
                # 这是通知（session/update、request_permission 等）
                self.notifications.put(msg)

    def _read_stderr(self):
        for line in self.p.stderr:
            self.stderr_lines.append(line.rstrip())

    def request(self, method, params=None, timeout=180):
        with self._lock:
            rid = self.next_id
            self.next_id += 1
            q: "queue.Queue" = queue.Queue()
            self.responses[rid] = q
        msg = {"jsonrpc": "2.0", "id": rid, "method": method}
        if params is not None:
            msg["params"] = params
        self.p.stdin.write(json.dumps(msg) + "\n")
        self.p.stdin.flush()
        try:
            return q.get(timeout=timeout)
        except queue.Empty:
            raise TimeoutError(f"{method} 超时（{timeout}s）")
        finally:
            self.responses.pop(rid, None)

    def drain_notifications(self, wait=0.0):
        out = []
        deadline = time.time() + wait
        while True:
            try:
                out.append(self.notifications.get_nowait())
            except queue.Empty:
                if time.time() >= deadline:
                    break
                time.sleep(0.02)
        return out


class Client:
    """ACP 客户端：initialize + session/new + session/prompt，带流式文本收集。"""

    def __init__(self, conn):
        self.conn = conn
        self.session_id = None
        self.agent_text = ""
        self.notifications = 0

    def initialize(self):
        return self.conn.request("initialize", {
            "protocolVersion": 1,
            "clientCapabilities": {
                "fs": {"readTextFile": False, "writeTextFile": False},
                "terminal": False,
            },
            "clientInfo": {"name": "diemeng-probe", "version": "0.1.0"},
        })

    def new_session(self, cwd):
        resp = self.conn.request("session/new", {"cwd": cwd, "mcpServers": []})
        if "error" in resp:
            return resp
        self.session_id = resp["result"]["sessionId"]
        return resp

    def prompt(self, text, timeout=300):
        """发任务并边收通知边收集文本。"""
        # 后台线程：持续把通知里的文本拼起来
        stop = threading.Event()

        def collect():
            while not stop.is_set():
                try:
                    msg = self.conn.notifications.get(timeout=0.2)
                except queue.Empty:
                    continue
                self.notifications += 1
                method = msg.get("method", "")
                if method == "session/update":
                    u = (msg.get("params") or {}).get("update") or {}
                    if u.get("sessionUpdate") == "agent_message_chunk":
                        c = u.get("content") or {}
                        if c.get("type") == "text":
                            self.agent_text += c.get("text", "")

        th = threading.Thread(target=collect, daemon=True)
        th.start()
        try:
            resp = self.conn.request("session/prompt", {
                "sessionId": self.session_id,
                "prompt": [{"type": "text", "text": text}],
            }, timeout=timeout)
        finally:
            time.sleep(0.15)  # 让最后几条通知落袋
            stop.set()
            th.join(timeout=2)
        return resp


def main():
    prompt = DEFAULT_PROMPT
    argv = sys.argv[1:]
    if "--prompt" in argv:
        prompt = argv[argv.index("--prompt") + 1]

    cwd = "/home/wings/Code_Project/diemeng"
    t0 = time.time()
    conn = Conn(["dsh", "--profile", "acp"])
    client = Client(conn)
    out = {"steps": {}, "cwd": cwd}

    init = client.initialize()
    out["steps"]["initialize"] = init
    if "error" in init:
        print(json.dumps(out, ensure_ascii=False, indent=2))
        return 1
    t_init = time.time() - t0

    sess = client.new_session(cwd)
    out["steps"]["session_new"] = sess
    if "error" in sess:
        print(json.dumps(out, ensure_ascii=False, indent=2))
        return 1
    out["session_id"] = client.session_id
    t_sess = time.time() - t0

    t1 = time.time()
    resp = client.prompt(prompt)
    t_prompt = time.time() - t1
    out["steps"]["session_prompt"] = resp
    out["init_ms"] = int(t_init * 1000)
    out["session_new_ms"] = int(t_sess * 1000)
    out["prompt_ms"] = int(t_prompt * 1000)
    out["agent_text"] = client.agent_text
    out["agent_text_len"] = len(client.agent_text)
    out["notifications"] = client.notifications
    out["stop_reason"] = (resp.get("result") or {}).get("stopReason")
    out["stderr_tail"] = conn.stderr_lines[-8:]
    conn.p.terminate()
    print(json.dumps(out, ensure_ascii=False, indent=2))
    return 0 if (out["agent_text_len"] > 0 and out["stop_reason"] == "end_turn") else 2


if __name__ == "__main__":
    sys.exit(main())
