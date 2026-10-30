<h1 align="center">Local-AI-Route</h1>

<p align="center">
  <a href="README.md">简体中文</a> · <b>English</b>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Rust-black?style=flat-square&logo=rust&logoColor=white" alt="Rust" />
  <img src="https://img.shields.io/badge/Tauri_v2-24C8D8?style=flat-square&logo=tauri&logoColor=white" alt="Tauri v2" />
  <img src="https://img.shields.io/badge/Vue_3-4FC08D?style=flat-square&logo=vue.js&logoColor=white" alt="Vue 3" />
  <img src="https://img.shields.io/badge/Tailwind_CSS-38B2AC?style=flat-square&logo=tailwind-css&logoColor=white" alt="Tailwind CSS" />
  <img src="https://img.shields.io/badge/Windows-0078D6?style=flat-square&logo=windows&logoColor=white" alt="Windows" />
  <img src="https://img.shields.io/badge/macOS-000000?style=flat-square&logo=apple&logoColor=white" alt="macOS" />
  <img src="https://img.shields.io/badge/Linux-FCC624?style=flat-square&logo=linux&logoColor=black" alt="Linux" />
  <img src="https://img.shields.io/badge/License-MIT-yellow.svg?style=flat-square" alt="MIT License" />
</p>

**Local-AI-Route** is a lightweight, zero-global-TUN, process-scoped proxy launcher and four-dimensional connection diagnostics tool designed for desktop AI applications. It natively supports:

- 🤖 **ChatGPT** (compatible with Windows Store/standalone, macOS App Bundles, and Linux desktop clients)
- 🧠 **Claude** (compatible with Windows MSIX/Squirrel, macOS App Bundles, and Linux desktop clients)
- 🌌 **Antigravity Desktop**
- ⚡ **Antigravity IDE** (strict app disambiguation & dedicated LSP proxy inheritance checks)
- 🧩 **Custom Extensible**: One-click addition of any Chromium / Electron-based desktop AI app (e.g. Cursor, Windsurf)

> [!NOTE]
> - This tool does not provide any proxy servers or network services. It merely injects your running local proxy (e.g. Clash / Mihomo / FlClash / v2rayN) into compatible desktop AI applications via process arguments and environment variables.
> - **Cross-Platform Support**: Natively adapted for Windows, macOS (Apple Silicon & Intel), and major Linux distributions with process-level proxy injection, signal management, 4D connection diagnostics, and desktop shortcut generation.

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

### Option 1: Download from Releases (Ready-to-Use)

Grab the appropriate package for your system from [GitHub Releases](../../releases):
- 🪟 **Windows**: Download and extract the portable ZIP `Local-AI-Route-<version>-windows-x64-portable.zip` or install via `Local-AI-Route_<version>_x64-setup.exe`.
- 🍎 **macOS**: Download `Local-AI-Route_<version>_universal.dmg` (Universal binary with native Apple Silicon M-series & Intel support) and drag to Applications.
- 🐧 **Linux**: Download the `.deb` package (`sudo dpkg -i ...`) or standalone `.AppImage` (`chmod +x` and run directly).

Usage:
1. Ensure your local proxy client (Clash, Mihomo, FlClash, v2rayN, etc.) is active.
2. Launch `Local-AI-Route` and click **"Scan Local Ports"** to detect your local proxy port (or configure manually and click **"Save Config"**).
3. Click **"Launch"** on the desired application card (or generate a **"Desktop Shortcut"** to start the app with proxy directly from your desktop).

### Option 2: Build from Source (Rust & Node.js)

#### Prerequisites
- **Rust Toolchain**: `stable`
- **Node.js**: v18+ & **pnpm**: v9+
- **Linux Build Dependencies (Ubuntu/Debian)**:
  ```bash
  sudo apt-get install -y libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf libxdo-dev libfuse2
  ```

#### Commands
```bash
# 1. Install frontend dependencies
pnpm install

# 2. Start dev mode (Hot-reload UI + Rust backend)
pnpm tauri dev

# 3. Build production package
# Windows: build standalone portable release package
.\scripts\build_windows.ps1

# macOS / Linux: build native packages (.dmg / .app / .deb / .AppImage)
pnpm tauri build
```

---

## Acknowledgements & License

- **Upstream Inspiration**: Architectural concepts and multi-channel discovery mechanisms are inspired by [oliver804/Local-Route-Launcher](https://github.com/oliver804/Local-Route-Launcher), [DSDS-CMHL/EasyAntigravity](https://github.com/DSDS-CMHL/EasyAntigravity), and [king-sheol/claude-desktop-windows-proxy-launcher](https://github.com/king-sheol/claude-desktop-windows-proxy-launcher). See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for details.
- **Local Zero-Telemetry**: No tracking, no data collection.
- **License**: [MIT License](LICENSE)
