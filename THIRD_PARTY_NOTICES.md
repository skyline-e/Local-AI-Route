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

4. **agy-switch (`anglee0323/agy-switch`)**
   - Repository: https://github.com/anglee0323/agy-switch
   - License: MIT License
   - Referenced concepts: Antigravity multi-account credential management with Windows Credential Manager (`gemini:antigravity`), CLI session tokens (`~/.gemini/antigravity-cli/antigravity-oauth-token`), Google OAuth token refresh flows, and Cloud Code model quota retrieval (`fetchAvailableModels`).

5. **codex-account-switcher (`liuzhao1225/codex-account-switcher`)**
   - Repository: https://github.com/liuzhao1225/codex-account-switcher
   - License: MIT License
   - Referenced concepts: ChatGPT / Codex multi-account configuration structure (`~/.codex/auth.json`), JWT identity and subscription claim decoding (`chatgpt_plan_type`, expiration), and 5-hour rate limit and weekly usage window normalization.

6. **claude-account-switcher (`SnlperStripes/claude-account-switcher`)**
   - Repository: https://github.com/SnlperStripes/claude-account-switcher
   - License: MIT License
   - Referenced concepts: Claude Desktop multi-account profile isolation, Windows DPAPI + AES-256-GCM Chromium v10 encrypted token cache extraction (`oauth:tokenCacheV2`), Network Cookies store backup and restoration, Anthropic OAuth usage querying (`/api/oauth/usage` with beta header), and fresh login state preparation.

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
