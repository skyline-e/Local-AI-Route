# Local-AI-Route

<p align="center">
  <a href="README.md">简体中文</a> · <b>English</b>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Rust-black?style=flat-square&logo=rust&logoColor=white" alt="Rust" />
  <img src="https://img.shields.io/badge/Tauri_v2-24C8D8?style=flat-square&logo=tauri&logoColor=white" alt="Tauri v2" />
  <img src="https://img.shields.io/badge/Vue_3-4FC08D?style=flat-square&logo=vue.js&logoColor=white" alt="Vue 3" />
  <img src="https://img.shields.io/badge/Tailwind_CSS-38B2AC?style=flat-square&logo=tailwind-css&logoColor=white" alt="Tailwind CSS" />
  <img src="https://img.shields.io/badge/Windows-0078D6?style=flat-square&logo=windows&logoColor=white" alt="Windows" />
  <img src="https://img.shields.io/badge/License-MIT-yellow.svg?style=flat-square" alt="MIT License" />
</p>

**Local-AI-Route** is a lightweight, zero-global-TUN, process-scoped proxy launcher and four-dimensional connection diagnostics tool designed for desktop AI applications. It natively supports:

- 🤖 **ChatGPT** (compatible with Microsoft Store and standalone packages)
- 🧠 **Claude** (compatible with MSIX, Squirrel incremental installs, and standard packages)
- 🌌 **Antigravity Desktop**
- ⚡ **Antigravity IDE** (strict registry disambiguation & dedicated LSP proxy inheritance checks)
- 🧩 **Custom Extensible**: One-click addition of any Chromium / Electron-based desktop AI app (e.g. Cursor, Windsurf)

> [!NOTE]
> - This tool does not provide any proxy servers or network services. It merely injects your running local proxy (e.g. Clash / Mihomo / FlClash / v2rayN) into compatible desktop AI applications via process arguments and environment variables.
> - **Platform Notice**: Currently, only the Windows platform has been tested and verified. If you wish to use it on other platforms (macOS / Linux), please compile from source yourself.

<p align="center">
  <img src="docs/image.png" alt="Local-AI-Route Preview" width="850" />
</p>

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
