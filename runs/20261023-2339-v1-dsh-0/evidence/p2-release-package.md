# 验收 P5 · 安装包（阶段 5 完成）

**日期**：2026-10-24 05:25 (+08:00)
**机器**：wings-pc
**命令**：`cd app && cargo tauri build`

## 结果

```
Bundling 蝶梦_0.1.0_amd64.deb
Finished 1 bundle at:
  target/release/bundle/deb/蝶梦_0.1.0_amd64.deb (6.20 MiB)
```

| 指标 | 实测 | 验收要求（P5） | 结论 |
|---|---|---|---|
| **deb 安装包** | **6.20 MiB** | <100MB / 理想 <40MB | ✅✅ **远优于理想值** |
| release 二进制 | 18MB | — | — |
| strip 后二进制 | 13MB | — | — |

## 为什么只出 deb，不出 AppImage

先试了 AppImage，**112MB，超了 12MB**。解剖后发现：

```
95M  lib/libwebkit2gtk-4.1.so.0
40M  lib/libjavascriptcoregtk-4.1.so.0
32M  lib/libicudata.so.78
22M  bin/diemeng-app
```

AppImage 是「全自包含」，把整个 WebKitGTK 运行时打进去了。
deb 只依赖系统包（`libwebkit2gtk-4.1-0`、`libgtk-3-0`），所以只有 6.2MB。

**决策**：Linux 桌面分发走 deb。
理由：任何装了桌面环境的 Linux 都已经有 WebKitGTK，没必要重复打包 95MB。
如果将来要发便携版，AppImage 可以作为「兜底」，但验收表里应标注它超重。

## 安装与运行

```bash
sudo dpkg -i 蝶梦_0.1.0_amd64.deb
diemeng-app          # 或从应用菜单启动「蝶梦」
```

## 还没验证的

- **没实际 `dpkg -i` 装过**（需要 sudo，本机没有）
- 没在干净系统上验证依赖解析
- 打包后没跑过冒烟测试

这几条属于「发布前最后一公里」，需要用户在自己的机器上装一次确认。
