# Third-Party Notices & Upstream Attribution

**Local-AI-Route** is an independent Rust + Tauri v2 + Vue 3 implementation inspired by and synthesizing architectural concepts from the following MIT-licensed open-source projects:

1. **Local Route Launcher (`oliver804/Local-Route-Launcher`)**
   - Repository: https://github.com/oliver804/Local-Route-Launcher
   - Copyright (c) 2026 Oliver
   - License: MIT License
   - Referenced concepts: Process-scoped Chromium `--proxy-server` / `--disable-quic` launch parameters, unauthenticated tunnel probe design, process-tree TCP connection snapshot scoping, and user-path redaction principles.

2. **EasyAntigravity (`DSDS-CMHL/EasyAntigravity`)**
   - Repository: https://github.com/DSDS-CMHL/EasyAntigravity
   - Copyright (c) EasyAntigravity Contributors
   - License: MIT License
   - Referenced concepts: Dual-target discovery and registry disambiguation between Antigravity Desktop (Hub) and Antigravity IDE, local listening proxy port scanning, `language_server` child process identification, and portable `data/` storage convention.

3. **Claude Desktop Windows Proxy Launcher (`king-sheol/claude-desktop-windows-proxy-launcher`)**
   - Repository: https://github.com/king-sheol/claude-desktop-windows-proxy-launcher
   - Copyright (c) 2026 king-sheol
   - License: MIT License
   - Referenced concepts: Multi-channel `Claude.exe` executable discovery across MSIX/AppX package repositories, Squirrel versioned directories (`%LOCALAPPDATA%\AnthropicClaude\app-*`), standard installation paths, and Windows Uninstall registry keys; working directory alignment on launch; and terminating existing unproxied single-instance processes (`-RestartExisting`) prior to relaunching with proxy arguments (while omitting legacy forced `--disable-gpu` flags to preserve hardware acceleration).

---

## Core Open-Source Dependencies

- **Tauri v2** — Apache-2.0 OR MIT License (https://github.com/tauri-apps/tauri)
- **Vue 3** — MIT License (https://github.com/vuejs/core)
- **Tailwind CSS** — MIT License (https://github.com/tailwindlabs/tailwindcss)
- **Lucide Icons (`lucide-vue-next`)** — ISC License (https://github.com/lucide-icons/lucide)
- **Tokio** — MIT License (https://github.com/tokio-rs/tokio)
- **Reqwest** — Apache-2.0 OR MIT License (https://github.com/seanmonstar/reqwest)
- **sysinfo** — MIT License (https://github.com/GuillaumeGomez/sysinfo)
- **windows-rs** — Apache-2.0 OR MIT License (https://github.com/microsoft/windows-rs)
- **winreg** — MIT License (https://github.com/gentoo90/winreg-rs)
- **mslnk** — MIT License (https://github.com/lyckantropen/mslnk)
