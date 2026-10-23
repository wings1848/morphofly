#!/usr/bin/env python3
"""阶段0 接缝 0.6/0.7 探针：用 stdio JSON-RPC 连 `dsh --profile acp`，验证
  - initialize 握手
  - session/new 建会话
  - session/prompt 派一个「列目录」任务并收到文本流
  - session/cancel 取消
输出 JSON 到 stdout，供 evidence 存档。
"""
import json, subprocess, sys, threading, time, queue

PROMPT = "请执行一个很小的任务：运行 `ls /home/wings/Code_Project/diemeng`，然后用一句话告诉我目录里有哪些文件。不要修改任何文件。"

class Conn:
    def __init__(self, cmd):
        self.p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE, text=True, bufsize=1)
        self.q = queue.Queue()
        self.next_id = 1
        self.stderr_lines = []
        threading.Thread(target=self._read, daemon=True).start()
        threading.Thread(target=self._read_err, daemon=True).start()

    def _read(self):
        for line in self.p.stdout:
            line = line.strip()
            if not line:
                continue
            try:
                self.q.put(json.loads(line))
            except json.JSONDecodeError:
                self.q.put({"__raw__": line})

    def _read_err(self):
        for line in self.p.stderr:
            self.stderr_lines.append(line.rstrip())

    def send(self, method, params=None, is_notification=False):
        msg = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            msg["params"] = params
        if not is_notification:
            msg["id"] = self.next_id
            self.next_id += 1
        self.p.stdin.write(json.dumps(msg) + "\n")
        self.p.stdin.flush()
        return msg.get("id")

    def request(self, method, params=None, timeout=180):
        rid = self.send(method, params)
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                m = self.q.get(timeout=0.5)
            except queue.Empty:
                if self.p.poll() is not None:
                    raise RuntimeError("dsh 进程退出: " + "\n".join(self.stderr_lines[-5:]))
                continue
            if m.get("id") == rid and ("result" in m or "error" in m):
                return m
            self.notes.append(m)
        raise TimeoutError(f"{method} 超时")

    notes = []

def main():
    t0 = time.time()
    conn = Conn(["dsh", "--profile", "acp"])
    conn.notes = []
    out = {"steps": {}}

    init = conn.request("initialize", {
        "protocolVersion": 1,
        "clientCapabilities": {"fs": {"readTextFile": False, "writeTextFile": False}, "terminal": False},
        "clientInfo": {"name": "diemeng-probe", "version": "0.0.1"},
    })
    out["steps"]["initialize"] = init
    if "error" in init:
        print(json.dumps(out, ensure_ascii=False, indent=2)); return 1

    t_init = time.time() - t0
    sess = conn.request("session/new", {
        "cwd": "/home/wings/Code_Project/diemeng",
        "mcpServers": [],
    })
    out["steps"]["session_new"] = sess
    if "error" in sess:
        print(json.dumps(out, ensure_ascii=False, indent=2)); return 1
    sid = sess["result"]["sessionId"]

    t1 = time.time()
    # 后台线程收 session/update 通知
    updates = []
    stop = threading.Event()
    def collect():
        while not stop.is_set():
            try:
                m = conn.q.get(timeout=0.3)
            except queue.Empty:
                continue
            updates.append(m)
    th = threading.Thread(target=collect, daemon=True); th.start()

    prompt = conn.request("session/prompt", {
        "sessionId": sid,
        "prompt": [{"type": "text", "text": PROMPT}],
    }, timeout=300)
    t_prompt = time.time() - t1
    stop.set(); th.join(timeout=2)
    out["steps"]["session_prompt"] = prompt
    out["first_response_ms"] = int(t_init * 1000)
    out["prompt_ms"] = int(t_prompt * 1000)
    # 抽出文本
    texts = []
    for m in updates:
        u = (m.get("params") or {}).get("update") or {}
        if u.get("sessionUpdate") == "agent_message_chunk":
            c = u.get("content") or {}
            if c.get("type") == "text":
                texts.append(c.get("text", ""))
    out["agent_text"] = "".join(texts)
    out["n_updates"] = len(updates)
    out["stopReason"] = (prompt.get("result") or {}).get("stopReason")
    out["stderr_tail"] = conn.stderr_lines[-8:]
    conn.p.terminate()
    print(json.dumps(out, ensure_ascii=False, indent=2))
    return 0

if __name__ == "__main__":
    sys.exit(main())
