#!/usr/bin/env bash
# 统一拿 wings-gpd 的 SSH 目标（与 pc_ssh.sh 同一套兜底策略）。
#
# wings-gpd 经常离线（掌机不进桌面就不联网），所以这个脚本只负责「当前能不能连」，
# 不负责等待。等待逻辑见 tools/gpd_wait_and_send.sh。
set -uo pipefail
if [ -n "${MORPHOFLY_GPD_SSH:-}" ]; then echo "$MORPHOFLY_GPD_SSH"; exit 0; fi
if getent hosts wings-gpd >/dev/null 2>&1; then echo "wings-gpd"; exit 0; fi
for ip in 100.107.9.102 192.168.31.10; do
  if timeout 4 bash -c "cat < /dev/null > /dev/tcp/$ip/22" 2>/dev/null; then echo "wings@$ip"; exit 0; fi
done
echo "ERROR: 找不到 wings-gpd（主机名和两个 IP 都不通）" >&2
exit 1
