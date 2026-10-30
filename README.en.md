# Local-AI-Route

<p align="center">
  <a href="README.md">简体中文</a> · <b>English</b> · <a href="docs/PLAN.md">Project Plan</a>
</p>

**Local-AI-Route** is a lightweight, zero-global-TUN, process-scoped proxy launcher and four-dimensional connection diagnostics tool designed for desktop AI applications. Built on a modern **Rust + Tauri v2 + Vue 3 / Tailwind CSS** architecture, it delivers sub-second startup (<0.3s), a ~15MB distribution size, and high-reliability, crash-free OS integration.

> This tool does not provide any proxy servers or network services. It merely injects your running local proxy (e.g. Clash / Mihomo / FlClash / v2rayN) into compatible desktop AI applications via process arguments and environment variables.

---

## Key Features

- **First-Class Presets + Custom App Extensions**:
  - Out-of-the-box auto-detection for **ChatGPT APP** (compatible with Microsoft Store `OpenAI.ChatGPT` / `OpenAI.Codex` MSIX and standalone installers), **Claude APP** (compatible with MSIX packages, `%LOCALAPPDATA%\AnthropicClaude\app-*` Squirrel installs, and standard installers), **Antigravity Desktop**, and **Antigravity IDE** (with strict registry disambiguation).
  - One-click addition of any Chromium / Electron-based desktop AI app (e.g. Cursor, Windsurf).
- **Concurrent Multi-App Management**:
  - Unified top card for global proxy settings (`HTTP` & `SOCKS5`, local port auto-scanning, editable `NO_PROXY` bypass rules).
  - Parallel app cards with independent **"Proxied" / "Direct"** switching.
- **Smart Conflict Detection & Staged Safe Restart**:
  - Identifies direct or outdated proxy instances; sends graceful close (`WM_CLOSE`) signals first, with safe tree termination confirmation if minimized to tray.
- **Four-Dimensional Diagnostics Engine (Redacted Export)**:
  1. **Tunnel Probe**: Lightweight reachability check via configured proxy to `chatgpt.com` or Google endpoints.
  2. **Main Process CLI Args**: Live verification of `--proxy-server` and `--disable-quic`.
  3. **Subprocess Env Check**: Antigravity-tailored verification of `language_server` `HTTPS_PROXY` / `HTTP_PROXY`.
  4. **Process-Tree TCP Snapshot**: Local proxy connection snapshot within the target PID tree with automated `%USERPROFILE%` redaction.
- **System Tray Residency & Desktop Shortcuts**:
  - Single instance, minimize to tray on close, autostart on boot, `--launch <app_id>` silent CLI launching, and desktop `.lnk` generation.
- **Dual-Storage Modes & Modern UI**:
  - Auto-switches between portable `data/` and standard `%LOCALAPPDATA%` modes.
  - Bilingual (zh-CN / en-US) with System / Light / Dark theme support.

---

## Quick Start

### Option 1: Green Portable ZIP (Recommended)

1. Unzip `Local-AI-Route-<version>-windows-x64-portable.zip` to any folder.
2. Launch `Local-AI-Route.exe` while your local proxy client is running.
3. Click **"Scan Local Ports"** to select your proxy port, then click **"Launch"** on the target card.

### Option 2: Build from Source (Rust & Node.js)

```powershell
# 1. Install frontend dependencies
pnpm install

# 2. Start dev mode (Hot-reload UI + Rust backend)
pnpm tauri dev

# 3. Build standalone Windows x64 release package
.\scripts\build_windows.ps1
```

---

## Acknowledgements & License

- **Upstream Inspiration**: Architectural concepts and multi-channel discovery mechanisms are inspired by [oliver804/Local-Route-Launcher](https://github.com/oliver804/Local-Route-Launcher), [DSDS-CMHL/EasyAntigravity](https://github.com/DSDS-CMHL/EasyAntigravity), and [king-sheol/claude-desktop-windows-proxy-launcher](https://github.com/king-sheol/claude-desktop-windows-proxy-launcher). See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for details.
- **Local Zero-Telemetry**: No tracking, no data collection.
- **License**: [MIT License](LICENSE)
