# Third-Party Notices & Upstream Attribution

**Local-AI-Route** is an independent Python implementation inspired by and synthesizing architectural concepts from the following MIT-licensed open-source projects:

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

---

## Runtime Python Dependencies

- **CustomTkinter** — MIT License (https://github.com/TomSchimansky/CustomTkinter)
- **pystray** — LGPLv3 License (https://github.com/moses-palmer/pystray)
- **Pillow (PIL Fork)** — MIT-CMU License (https://github.com/python-pillow/Pillow)
- **psutil** — BSD-3-Clause License (https://github.com/giampaolo/psutil)
- **HTTPX & socksio** — BSD-3-Clause License (https://github.com/encode/httpx)
