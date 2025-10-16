# clipfixd

一个修复 Linux 下 X11 和 Wayland 应用之间剪贴板兼容性问题的后台守护进程。

## 1. 它做什么？

clipfixd 监控 X11 和 Wayland 剪贴板，自动转换不兼容的剪贴板格式，以提升跨环境的兼容性。

尤其是针对某些采用错误或落后逻辑实现剪贴板的 Electron 和 X11 应用（如 QQ、WPS、飞书）。

相关技术细节可见 [该 Gist](https://gist.github.com/w568w/3b180b19cff4325fcf457bc77cd5fa8b)。

## 2. 安装

### 2.1. 从源码构建

```bash
cargo build --release
```

生成的二进制文件位于 `target/release/clipfixd`。

### 2.2. 安装到系统

```bash
cargo install --git https://github.com/w568w/clipfixd.git
```

或手动复制二进制文件：

```bash
cp target/release/clipfixd ~/.cargo/bin/
```

## 3. 使用

### 3.1. 手动运行

```bash
clipfixd
```

守护进程将在前台运行。按 Ctrl+C 停止。

### 3.2. 作为 systemd 用户服务运行

1. 复制服务文件：

```bash
mkdir -p ~/.config/systemd/user/
cp clipfixd.service ~/.config/systemd/user/
```

2. 启用并启动服务：

```bash
systemctl --user enable clipfixd.service
systemctl --user start clipfixd.service
```

3. 查看服务状态：

```bash
systemctl --user status clipfixd.service
```

4. 查看日志：

```bash
journalctl --user -u clipfixd.service -f
```

## 4. 需求

- 同时支持 X11 和 Wayland 的 Linux 桌面环境
- Wayland 混成器支持 [`wlr-data-control-unstable-v1`](https://wayland.app/protocols/wlr-data-control-unstable-v1)，如 KWin、Sway、Hyprland 等
- 你确实遇到过剪贴板问题！

## 5. 技术细节

clipfixd 运行两个线程：

- **X11 监听器**：使用 `x11-clipboard` 监控 X11 剪贴板
- **Wayland 监听器**：使用 `wayland-clipboard-listener` 监控 Wayland 剪贴板

当检测到特定 MIME 类型时，执行格式转换并更新另一个剪贴板系统。

## 6. 已知并修复的问题

- QQ (X11) 复制文件到 KDE 应用 (Wayland) 和第三方应用 (Wayland) 时，无法正确复制
- Spectacle/Chromium (Wayland) 复制图片到 QQ/WPS (X11) 时，卡顿或图片残缺

## 7. 许可

由于依赖要求，本应用在 **GNU 通用公共许可证，版本 3.0** 下发布。
