# 编译走 GitHub Actions（gpd 上不再本地编译）

**2026-10-25** ｜ 用户要求：「记住编译走 github actions」

## 为什么

- wings-gpd 是掌机（Pentium N6000，7.6GB），本地 `cargo build --release -p morphofly-app`
  要 5–13 分钟，编译期间 app 和 Chrome 都卡，而且和「边跑边测」抢 CPU。
- CI 有缓存（Swatinem/rust-cache），机器比掌机快；编出来的二进制顺手留档，
  哪个提交对应哪个程序一目了然。

## 流程

```
本地改代码 → git push → GitHub Actions ci.yml
                              ├─ test / test-kws / build-windows / secret-scan（照旧）
                              └─ build-app-linux  ← 新增：编出可下载产物
                                   artifact: morphofly-app-linux-x64.tar.gz
                                     ├─ morphofly-app（二进制）
                                     ├─ resources/（唤醒词模型、VAD、关键词表）
                                     └─ build.txt（sha + 构建时间）
        ↓
tools/gpd_deploy_ci.sh   ← 下载最新成功产物 → 传到 gpd → 解到
                            ~/Code_Project/morphofly/target/release/ → 重启服务
        ↓
真机验证（tools/verify_float_ui.sh / tools/gpd_voice_e2e.sh）
```

部署命令：

```bash
tools/gpd_deploy_ci.sh                 # 取 main 上最近一次成功 CI
tools/gpd_deploy_ci.sh --sha <commit>  # 指定提交
tools/gpd_deploy_ci.sh --dry-run       # 只看会做什么
```

## 注意

- 产物解到 `target/release/`（服务里 `MORPHOFLY_BUNDLE_DIR` 已经指向
  `target/release/resources`），所以**服务配置不用改**；
  但以后谁再在 gpd 上本地编，会把这个目录覆盖掉。
- gpd 上只跑 CI 产物；本地编译只作为「CI 挂了想快速看一眼」的最后手段，用完记得重新部署 CI 产物。
- CI 的 Linux 构建是 ubuntu-24.04（glibc 2.39），gpd 是 Arch 系（glibc 更新），
  向下兼容没问题；如果以后要给更老的发行版用，把 job 换成 ubuntu-22.04。
