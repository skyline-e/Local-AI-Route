# Local-AI-Route

<p align="center">
  <b>简体中文</b> · <a href="README.en.md">English</a> · <a href="docs/PLAN.md">项目计划书</a>
</p>

**Local-AI-Route** 是一款面向桌面 AI 应用的轻量级、免全局 TUN、免改系统代理的**进程级本地代理启动与四维连接诊断工具**，基于 **Rust + Tauri v2 + Vue 3 / Tailwind CSS** 现代原生架构开发，兼具极致的启动速度（<0.3s）、极小的安装体积（~15MB）与高可靠的零崩溃系统级掌控能力。

> 本工具不提供任何代理节点或网络接入服务，仅将您本机已运行的本地代理（如 Clash / Mihomo / FlClash / v2rayN）按进程级参数与环境变量注入兼容的桌面 AI 应用。

---

## 核心特性

- **四大一等公民预设 + 自定义应用扩展**：
  - 开箱自动定位 **ChatGPT APP**（兼容 Microsoft Store `OpenAI.ChatGPT` / `OpenAI.Codex` MSIX 商店版与独立安装包）、**Claude APP**（兼容 MSIX 商店版、`%LOCALAPPDATA%\AnthropicClaude\app-*` Squirrel 安装包及标准安装目录）、**Antigravity 桌面版**、**Antigravity IDE**（精确区分注册表条目防误判）。
  - 支持一键添加任意 Chromium / Electron 类桌面 AI 应用（如 Cursor、Windsurf 等）。
- **多应用并列并发管理**：
  - 顶部统一配置全局代理（支持 `HTTP` 与 `SOCKS5`、本地常见代理监听端口自动扫描填入、可编辑 `NO_PROXY` 本地/内网绕过规则），下方多卡片支持同时运行与各自独立的 **「走代理 / 直连」** 切换。
- **智能冲突识别与分级安全重启**：
  - 自动识别已在后台直连运行或代理端口已变更的实例；重启/停止优先发送优雅关闭信号（`WM_CLOSE`），若应用缩进托盘未退则弹窗由用户二次确认后清理残留进程树（对 Antigravity IDE 等编辑器额外给出未保存工作高危提醒）。
- **四维精细化连接诊断（脱敏导出）**：
  1. **代理隧道探测**：经本地配置代理向端点（`chatgpt.com` 或 Google 探测点）发起轻量探测，明确区分隧道可达与上游拦截。
  2. **主进程启动参数**：实时核验主进程命令行中的 `--proxy-server` 与 `--disable-quic`。
  3. **子进程环境变量继承**：针对 Antigravity 专项核查 `language_server` 子进程是否成功继承 `HTTPS_PROXY` / `HTTP_PROXY`。
  4. **进程树活跃 TCP 快照**：在目标应用进程树范围内快照指向代理端口的连接，一键复制脱敏诊断报告（自动将用户路径替换为 `%USERPROFILE%`）。
- **系统托盘驻留 + 桌面带代理快捷方式**：
  - 单实例运行，关闭窗口自动隐藏至系统托盘，支持开机静默驻留，支持 `--launch <app_id>` 命令行直启及一键生成带原应用图标的桌面快捷方式。
- **双存储模式与现代化外观**：
  - 自动识别同目录 `data/` 绿色便携模式与 `%LOCALAPPDATA%\Local-AI-Route` 标准模式；支持简体中文 / English 即时切换与跟随系统 / 浅色 / 深色外观。

---

## 快速开始

### 方式一：使用绿色便携包（推荐，开箱即用）

1. 下载解压 `Local-AI-Route-<version>-windows-x64-portable.zip` 到任意目录。
2. 确保本地代理客户端（Clash / Mihomo 等）已启动，运行 `Local-AI-Route.exe`。
3. 点击 **「扫描本地端口」** 快捷填入代理端口（或手动修改后点击 **「保存配置」**）。
4. 在对应卡片点击 **「启动」**（也可点击 **「桌面快捷方式」**，之后直接双击桌面图标静默带代理拉起应用）。

### 方式二：从源码运行与构建（Rust + Node.js）

#### 环境准备
- **Rust Toolchain**: `stable`（推荐 `stable-x86_64-pc-windows-gnu` 或 `msvc`）
- **Node.js**: v18+ & **pnpm**: v9+

```powershell
# 1. 安装前端依赖
pnpm install

# 2. 启动开发模式（热重载界面 + Rust 后端）
pnpm tauri dev

# 3. 构建 Windows x64 独立绿色发布包
.\scripts\build_windows.ps1
```

---

## 技术架构优势（对比传统 Python 方案）

| 维度 | 旧方案 (Python + CustomTkinter) | **现方案 (Rust + Tauri v2)** |
| :--- | :--- | :--- |
| **单文件 / 压缩包体积** | 80MB ~ 150MB | **~15MB (精简近 85%)** |
| **冷启动耗时** | 1~3 秒明显卡顿 | **< 0.3 秒瞬间秒开** |
| **内存占用 (托盘驻留)** | 80MB ~ 120MB | **30MB ~ 40MB** |
| **高 DPI / 多屏缩放** | 偶发文字发虚、布局撕裂 | **原生 WebKit/WebView2 矢量无损** |
| **杀毒软件误报率** | 高 (PyInstaller 通病) | **极低 (纯原生 PE 机器码)** |
| **进程与系统交互稳定性** | 容易因多线程竞争锁死 Tkinter | **Rust 所有权模型编译期保证并发安全** |

---

## 隐私与开源许可

- **本地零遥测**：无任何账号体系、无后台数据上报；连接诊断仅在用户主动点击时经本地配置的代理发起无凭据握手。
- **许可证**：[MIT License](LICENSE)
