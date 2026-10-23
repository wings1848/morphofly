#!/usr/bin/env bash
# 统一拿 wings-pc 的 SSH 目标。
# 2026-10-24: 本机 /etc/resolv.conf 被 dhcpcd 覆写，MagicDNS 失效，`wings-pc` 解析不到；
# 这里优先用 Tailscale IP 兜底，避免每次手工试。
set -euo pipefail
if [ -n "${DIEMENG_PC_SSH:-}" ]; then echo "$DIEMENG_PC_SSH"; exit 0; fi
if getent hosts wings-pc >/dev/null 2>&1; then echo "wings-pc"; exit 0; fi
for ip in 100.110.222.0 192.168.31.20; do
  if timeout 4 bash -c "cat < /dev/null > /dev/tcp/$ip/22" 2>/dev/null; then echo "wings@$ip"; exit 0; fi
done
echo "ERROR: 找不到 wings-pc（主机名和两个 IP 都不通）" >&2
exit 1
