# 进展

## 已完成

1. **仓库在 Windows 上根本检不出来** —— 根目录有个被跟踪的空文件叫 `...`，
   Windows 文件名不能以点结尾。删除 + 加 .gitignore。
2. **423MB 构建产物入库** —— `ci-dist/` 把 AppImage 和解开的 squashfs-root 提交了
   （400 文件，单个 libwebkit2gtk 86MB）。停止跟踪 + 加 .gitignore。
   历史里仍有 blob（.git 245MB），要彻底变小得改写历史（不可逆，待用户点头）。
3. **数据目录只读 HOME** → HOME/USERPROFILE/'.' 三级回退（+3 测试）。
4. **拉 dsh 不认 dsh.cmd** → spawn_program 按 PATHEXT 试 + `cmd /C` 兜底。
5. **平台代码隔离** → force_x11_backend cfg 到 linux。
6. **图标** → 补 icon.ico（256/128/64/48/32/16），bundle.icon 加进去。
7. **打包** → tauri.conf 补 windows.nsis 段；release 加 Windows job（--bundles nsis）。
8. **闸门** → CI 加 build-windows job。

## 待验证（CI 跑完填）

- W1~W5 的真实结果
