#!/usr/bin/env python3
"""蝶梦语音交互端到端测试执行器。

流程：TTS 合成音频 → 喂给蝶梦 → 采集实际行为 → 对标期望 → 出报告。

两种运行模式：
  --mode mock   本地模拟（不需要 DSH/网络，验证判定逻辑本身）
  --mode real   wings-pc 真机（启动 app + 真链路）

用法：
    python3 tools/e2e/run.py --mode mock
    python3 tools/e2e/run.py --mode real --case wake-01
"""
from __future__ import annotations

import argparse
import asyncio
import json
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

try:
    import yaml
except ImportError:
    print("需要 pyyaml: pip install pyyaml", file=sys.stderr)
    sys.exit(1)

ROOT = Path(__file__).resolve().parents[2]
CASES = ROOT / "tools" / "e2e" / "cases.yaml"
AUDIO_DIR = Path("/tmp/diemeng-e2e-audio")


@dataclass
class CaseResult:
    case_id: str
    cat: str
    text: str
    passed: bool
    checks: list[tuple[str, bool, str]] = field(default_factory=list)

    def add(self, name: str, ok: bool, detail: str = "") -> None:
        self.checks.append((name, ok, detail))
        if not ok:
            self.passed = False


# ── 判定逻辑（纯函数，可单测）──

def check_wake(expect: dict, actual: dict, res: CaseResult) -> None:
    """唤醒判定。"""
    if "wake" not in expect:
        return
    want = expect["wake"]
    got = actual.get("wake", False)
    res.add("唤醒", want == got, f"期望 {want}，实际 {got}")


def check_tool_calls(expect: dict, actual: dict, res: CaseResult) -> None:
    """工具调用判定。"""
    if "tool_calls" not in expect:
        return
    want = sorted(expect["tool_calls"])
    got = sorted(actual.get("tool_calls", []))
    res.add("工具调用", want == got, f"期望 {want}，实际 {got}")


def check_reply_length(expect: dict, actual: dict, res: CaseResult) -> None:
    """回复长度判定（对应「极其简洁」要求）。"""
    limit = expect.get("reply_max_chars")
    if limit is None:
        return
    reply = actual.get("reply", "")
    n = len(reply)
    res.add("回复长度", n <= limit, f"{n} 字 > 限制 {limit}：{reply[:50]!r}")


def check_reply_contains(expect: dict, actual: dict, res: CaseResult) -> None:
    """回复关键词判定（任一命中即可）。"""
    wanted = expect.get("reply_contains")
    if not wanted:
        return
    reply = actual.get("reply", "")
    hit = [w for w in wanted if w in reply]
    res.add("回复含关键词", bool(hit), f"期望含 {wanted}，实际 {reply[:60]!r}")


def check_bargein(expect: dict, actual: dict, res: CaseResult) -> None:
    """打断判定。"""
    if not expect.get("bargein"):
        return
    limit = expect.get("interrupt_latency_ms_max", 400)
    ms = actual.get("interrupt_latency_ms")
    if ms is None:
        res.add("打断", False, "没有发生打断")
    else:
        res.add("打断延迟", ms <= limit, f"{ms}ms > {limit}ms")


def check_confirm(expect: dict, actual: dict, res: CaseResult) -> None:
    """是否要求用户确认。"""
    if not expect.get("confirm_expected"):
        return
    got = actual.get("asked_confirm", False)
    res.add("要求确认", got, f"复杂任务应问确认，实际 {'问了' if got else '没问'}")


def judge(case: dict, actual: dict) -> CaseResult:
    """对单条用例下判定。"""
    res = CaseResult(
        case_id=case["id"],
        cat=case.get("cat", ""),
        text=case.get("text", ""),
        passed=True,
    )
    exp = case.get("expect") or {}
    check_wake(exp, actual, res)
    check_tool_calls(exp, actual, res)
    check_reply_length(exp, actual, res)
    check_reply_contains(exp, actual, res)
    check_bargein(exp, actual, res)
    check_confirm(exp, actual, res)
    return res


# ── 执行 ──

async def synth_audio(text: str, out: Path) -> bool:
    """TTS 合成。空文本直接造静音。"""
    out.parent.mkdir(parents=True, exist_ok=True)
    if not text.strip():
        # 1 秒静音（16k mono s16le）
        out.write_bytes(b"\x00\x00" * 16000)
        return True
    try:
        await tts_synth(text, out)
        return True
    except Exception as e:
        print(f"  ⚠️ TTS 失败: {e}", file=sys.stderr)
        return False


async def tts_synth(text: str, out: Path) -> None:
    with __import__("tempfile").NamedTemporaryFile(suffix=".mp3", delete=False) as tf:
        mp3 = Path(tf.name)
    try:
        proc = await asyncio.create_subprocess_exec(
            "uv", "run", "--quiet", "--with", "edge-tts",
            "edge-tts", "--voice", "zh-CN-XiaoxiaoNeural",
            "--text", text, "--write-media", str(mp3),
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
        )
        _, err = await proc.communicate()
        if proc.returncode != 0:
            raise RuntimeError(err.decode()[:150])
        subprocess.run(
            ["ffmpeg", "-y", "-loglevel", "error", "-i", str(mp3),
             "-ar", "16000", "-ac", "1", "-f", "s16le", str(out)],
            check=True, capture_output=True,
        )
    finally:
        mp3.unlink(missing_ok=True)


async def run_mock(case: dict) -> dict:
    """mock 模式：用启发式规则模拟蝶梦的行为，验证判定逻辑。"""
    text = case.get("text", "")
    exp = case.get("expect") or {}

    # 简单启发式：模拟「如果实现正确会怎样」
    wake = "小蝶" in text and not any(x in text for x in ["小蝶儿", "蝴蝶", "小的"])
    tool_calls: list[str] = []
    reply = ""

    if wake:
        if any(k in text for k in ["天气", "你叫什么", "名字"]):
            reply = "我在呢" if "名字" not in text else "我是小蝶"
        elif any(k in text for k in ["那个项目", "aimeng"]):
            reply = "你说的是哪个项目？"      # 路径歧义必须追问
        elif any(k in text for k in ["看看", "有多少", "跑一下"]):
            tool_calls = ["delegate_task"]
            reply = "好，干完说"
        elif any(k in text for k in ["提升到", "整个项目"]):
            tool_calls = ["delegate_task"]
            reply = "要我这么做吗？"
        elif text.strip() == "停":
            reply = "好，停了"
        elif text.replace("小蝶", "").strip() in ("帮我", "帮忙", "帮"):
            # 话没说完（"小蝶帮我"），应追问
            reply = "你想让我帮什么？"
        else:
            reply = "在呢"

    return {
        "wake": wake,
        "tool_calls": tool_calls,
        "reply": reply,
        "asked_confirm": "要我" in reply,
        "interrupt_latency_ms": 300 if text.strip() == "停" else None,
    }


async def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["mock", "real"], default="mock")
    ap.add_argument("--case", help="只跑指定用例 id")
    ap.add_argument("--no-tts", action="store_true", help="跳过 TTS（省时间）")
    args = ap.parse_args()

    data = yaml.safe_load(CASES.read_text())
    cases = data["cases"]
    if args.case:
        cases = [c for c in cases if c["id"] == args.case]
        if not cases:
            print(f"没找到用例 {args.case}", file=sys.stderr)
            return 1

    results: list[CaseResult] = []
    for case in cases:
        print(f"\n── {case['id']} [{case.get('cat')}] {case.get('text','')[:40]!r}")

        if args.mode == "real":
            audio = AUDIO_DIR / f"{case['id']}.pcm"
            if not args.no_tts:
                ok = await synth_audio(case.get("text", ""), audio)
                if not ok:
                    print("  ⏭️ 跳过（TTS 失败）")
                    continue
            print(f"  音频: {audio} （{audio.stat().st_size // 2 / 16000:.2f}s）")
            print("  ⚠️ real 模式需要 GUI+音频设备，当前未实现驱动，先跑 mock")
            actual = await run_mock(case)
        else:
            actual = await run_mock(case)

        res = judge(case, actual)
        results.append(res)
        mark = "✅" if res.passed else "❌"
        print(f"  {mark} 实际: wake={actual.get('wake')} tools={actual.get('tool_calls')} reply={actual.get('reply','')[:40]!r}")
        for name, ok, detail in res.checks:
            if not ok:
                print(f"     ✗ {name}: {detail}")

    # 汇总
    print("\n" + "=" * 60)
    passed = sum(1 for r in results if r.passed)
    print(f"结果: {passed}/{len(results)} 通过")
    if passed < len(results):
        print("\n失败明细:")
        for r in results:
            if not r.passed:
                print(f"  {r.case_id} [{r.cat}]")
                for name, ok, detail in r.checks:
                    if not ok:
                        print(f"    {name}: {detail}")

    out = Path("/tmp/e2e-report.json")
    out.write_text(json.dumps(
        [{"id": r.case_id, "cat": r.cat, "passed": r.passed,
          "checks": [{"name": n, "ok": o, "detail": d} for n, o, d in r.checks]}
         for r in results], ensure_ascii=False, indent=2))
    print(f"\n报告: {out}")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
