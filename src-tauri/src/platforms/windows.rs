#[cfg(target_os = "windows")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "windows")]
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
#[cfg(target_os = "windows")]
use winreg::RegKey;

#[cfg(target_os = "windows")]
pub fn resolve_executable_for_launch(path_str: &str) -> String {
    // 0. If path points inside a protected MSIX package folder (C:\Program Files\WindowsApps\...),
    // redirect to its user-accessible App Execution Alias (%LOCALAPPDATA%\Microsoft\WindowsApps\*.exe)
    // so CreateProcessW attaches WIN://SYSAPPID and avoids OS error 5 (Access Denied) on standard Windows systems.
    if let Some(alias) = resolve_msix_execution_alias(path_str) {
        return alias;
    }

    let p = Path::new(path_str);
    if parse_msix_package_from_path(path_str).is_some() && !p.is_file() {
        if let Some(latest_pkg_exe) = find_appx_exe_by_family_or_alias(path_str) {
            return resolve_msix_execution_alias(&latest_pkg_exe).unwrap_or(latest_pkg_exe);
        }
    }

    if p.is_dir() {
        // 1. Try finding squirrel app-* latest exe (known names first, then any main exe inside app-*)
        if let Some(sq) = find_latest_squirrel_exe(p, "Gemini.exe")
            .or_else(|| find_latest_squirrel_exe(p, "Claude.exe"))
            .or_else(|| find_latest_squirrel_exe(p, "Factory.exe"))
            .or_else(|| find_latest_any_squirrel_exe(p))
        {
            return sq;
        }

        // 2. Try dir_name.exe in p or p\app
        if let Some(dir_name) = p.file_name().and_then(|n| n.to_str()) {
            for candidate in [
                p.join(format!("{}.exe", dir_name)),
                p.join("app").join(format!("{}.exe", dir_name)),
            ] {
                if candidate.is_file() {
                    return candidate.to_string_lossy().to_string();
                }
            }
        }

        // 3. Try any executable in p or p\app directly (excluding uninstallers/updaters/helpers)
        for search_dir in [p.to_path_buf(), p.join("app")] {
            if let Ok(entries) = std::fs::read_dir(&search_dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let candidate = entry.path();
                    if candidate.is_file()
                        && candidate
                            .extension()
                            .map_or(false, |ext| ext.eq_ignore_ascii_case("exe"))
                    {
                        let fname_low = candidate
                            .file_name()
                            .and_then(|f| f.to_str())
                            .unwrap_or("")
                            .to_lowercase();
                        if !is_installer_or_helper_exe(&fname_low) {
                            return candidate.to_string_lossy().to_string();
                        }
                    }
                }
            }
        }
    } else if let Some(parent) = p.parent() {
        let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !fname.is_empty() {
            let parent_is_app_ver = parent
                .file_name()
                .and_then(|n| n.to_str())
                .map_or(false, |s| s.to_lowercase().starts_with("app-"));
            if parent_is_app_ver {
                // 若配置了 Squirrel 的版本化子目录（如 app-1.0.0\App.exe），自动解析到最新版本的 app-*\App.exe，
                // 防止应用自动更新并清理旧版本目录后启动失败。
                if let Some(squirrel_root) = parent.parent() {
                    if let Some(latest) = find_latest_squirrel_exe(squirrel_root, fname) {
                        return latest;
                    }
                }
            } else if p.is_file() {
                // 若用户选中或配置了 Squirrel 根目录启动桩（如 %LOCALAPPDATA%\Google\Gemini\Gemini.exe 约400KB），
                // 自动重定向至 app-*\ 子目录下的真实 Electron 运行实体（约240MB），确保命令行参数与环境直达注入。
                if let Some(latest) = find_latest_squirrel_exe(parent, fname) {
                    return latest;
                }
            }
        }
    }
    path_str.to_string()
}

#[cfg(target_os = "windows")]
pub fn discover_target_executable(app_id: &str) -> Option<String> {
    let found = discover_target_executable_raw(app_id)?;
    Some(resolve_msix_execution_alias(&found).unwrap_or(found))
}

#[cfg(target_os = "windows")]
fn discover_target_executable_raw(app_id: &str) -> Option<String> {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let prog_files = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
    let prog_files_x86 = std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into());

    match app_id {
        "chatgpt" => {
            // 1. Check Appx / MSIX Repository in HKCU and resolve to launchable Store Execution Alias
            if let Some(found) = find_latest_appx_exe(&["openai.chatgpt", "openai.codex"], "ChatGPT.exe") {
                if let Some(alias) = resolve_msix_execution_alias(&found) {
                    return Some(alias);
                }
            }

            // 2. Check Store Execution Alias directly in %LOCALAPPDATA%\Microsoft\WindowsApps
            if !local_app_data.is_empty() {
                for alias_name in &["ChatGPT.exe", "chatgpt-desktop.exe"] {
                    let alias = Path::new(&local_app_data)
                        .join("Microsoft")
                        .join("WindowsApps")
                        .join(alias_name);
                    if is_executable_or_alias(&alias) {
                        return Some(alias.to_string_lossy().to_string());
                    }
                }
            }

            // 3. Check standard Win32 paths & Uninstall registry
            if !local_app_data.is_empty() {
                let c = Path::new(&local_app_data)
                    .join("Programs")
                    .join("ChatGPT")
                    .join("ChatGPT.exe");
                if c.is_file() {
                    return Some(c.to_string_lossy().to_string());
                }
            }
            for base in &[&prog_files, &prog_files_x86] {
                if !base.is_empty() {
                    let c = Path::new(base).join("ChatGPT").join("ChatGPT.exe");
                    if c.is_file() {
                        return Some(c.to_string_lossy().to_string());
                    }
                }
            }
            if let Some(path) = search_uninstall_registry("chatgpt", false, "ChatGPT.exe") {
                return Some(path);
            }

            // 4. Fallback to raw Appx package binary if execution alias is disabled in system settings
            if let Some(found) = find_latest_appx_exe(&["openai.chatgpt", "openai.codex"], "ChatGPT.exe") {
                return Some(found);
            }
        }
        "claude" => {
            // 1. Check Appx / MSIX Repository in HKCU and resolve to launchable Store Execution Alias
            if let Some(found) = find_latest_appx_exe(&["claude_", "anthropic"], "Claude.exe") {
                if let Some(alias) = resolve_msix_execution_alias(&found) {
                    return Some(alias);
                }
            }

            // 2. Check Squirrel / Win32 install in %LOCALAPPDATA%\AnthropicClaude\app-*\Claude.exe
            if !local_app_data.is_empty() {
                let squirrel_root = Path::new(&local_app_data).join("AnthropicClaude");
                if let Some(found) = find_latest_squirrel_exe(&squirrel_root, "Claude.exe") {
                    return Some(found);
                }
                let squirrel_stub = squirrel_root.join("Claude.exe");
                if squirrel_stub.is_file() {
                    return Some(squirrel_stub.to_string_lossy().to_string());
                }

                // 3. Check standard LocalAppData Programs paths
                for sub in &[
                    r"Programs\Claude\Claude.exe",
                    r"Programs\Claude Desktop\Claude.exe",
                    r"Programs\claude-desktop\Claude.exe",
                ] {
                    let candidate = Path::new(&local_app_data).join(sub);
                    if candidate.is_file() {
                        return Some(candidate.to_string_lossy().to_string());
                    }
                }
            }

            // 4. Check ProgramFiles / ProgramFiles(x86) paths
            for base in &[&prog_files, &prog_files_x86] {
                if base.is_empty() {
                    continue;
                }
                for sub in &[
                    r"Anthropic\Claude\Claude.exe",
                    r"Claude\Claude.exe",
                    r"Claude Desktop\Claude.exe",
                ] {
                    let candidate = Path::new(base).join(sub);
                    if candidate.is_file() {
                        return Some(candidate.to_string_lossy().to_string());
                    }
                }
            }

            // 5. Check Uninstall registry
            if let Some(path) = search_uninstall_registry("claude", false, "Claude.exe") {
                return Some(path);
            }

            // 6. Check Windows Store Execution Alias
            if !local_app_data.is_empty() {
                for alias_name in &["Claude.exe", "claude-desktop.exe"] {
                    let alias = Path::new(&local_app_data)
                        .join("Microsoft")
                        .join("WindowsApps")
                        .join(alias_name);
                    if is_executable_or_alias(&alias) {
                        return Some(alias.to_string_lossy().to_string());
                    }
                }
            }

            // 7. Fallback to raw Appx package binary if execution alias is disabled in system settings
            if let Some(found) = find_latest_appx_exe(&["claude_", "anthropic"], "Claude.exe") {
                return Some(found);
            }
        }
        "gemini" => {
            // 1. Check Google\Gemini in %LOCALAPPDATA%
            if !local_app_data.is_empty() {
                let gemini_root = Path::new(&local_app_data).join("Google").join("Gemini");
                if let Some(found) = find_latest_squirrel_exe(&gemini_root, "Gemini.exe") {
                    return Some(found);
                }
                let stub = gemini_root.join("Gemini.exe");
                if stub.is_file() {
                    return Some(stub.to_string_lossy().to_string());
                }
                for sub in &[
                    r"Programs\Gemini\Gemini.exe",
                    r"Programs\Google\Gemini\Gemini.exe",
                ] {
                    let candidate = Path::new(&local_app_data).join(sub);
                    if candidate.is_file() {
                        return Some(candidate.to_string_lossy().to_string());
                    }
                }
            }

            // 2. Check ProgramFiles / ProgramFiles(x86)
            for base in &[&prog_files, &prog_files_x86] {
                if base.is_empty() {
                    continue;
                }
                for sub in &[
                    r"Google\Gemini\Gemini.exe",
                    r"Gemini\Gemini.exe",
                ] {
                    let candidate = Path::new(base).join(sub);
                    if candidate.is_file() {
                        return Some(candidate.to_string_lossy().to_string());
                    }
                }
            }

            // 3. Check Uninstall registry
            if let Some(path) = search_uninstall_registry("gemini", false, "Gemini.exe") {
                return Some(path);
            }
        }
        "antigravity-app" => {
            // 1. Search Uninstall keys
            if let Some(path) = search_uninstall_registry("antigravity", false, "Antigravity.exe") {
                return Some(path);
            }
            // 2. Check LocalAppData Programs
            if !local_app_data.is_empty() {
                let c = Path::new(&local_app_data)
                    .join("Programs")
                    .join("Antigravity")
                    .join("Antigravity.exe");
                if c.is_file() {
                    return Some(c.to_string_lossy().to_string());
                }
            }
            // 3. Check ProgramFiles / ProgramFiles(x86)
            for base in &[&prog_files, &prog_files_x86] {
                if !base.is_empty() {
                    let c = Path::new(base).join("Antigravity").join("Antigravity.exe");
                    if c.is_file() {
                        return Some(c.to_string_lossy().to_string());
                    }
                }
            }
        }
        "antigravity-ide" => {
            // 1. Search Uninstall keys
            if let Some(path) = search_uninstall_registry("antigravity ide", true, "Antigravity IDE.exe") {
                return Some(path);
            }
            // 2. Check LocalAppData Programs
            if !local_app_data.is_empty() {
                let c = Path::new(&local_app_data)
                    .join("Programs")
                    .join("Antigravity IDE")
                    .join("Antigravity IDE.exe");
                if c.is_file() {
                    return Some(c.to_string_lossy().to_string());
                }
            }
            // 3. Check ProgramFiles / ProgramFiles(x86)
            for base in &[&prog_files, &prog_files_x86] {
                if !base.is_empty() {
                    let c = Path::new(base).join("Antigravity IDE").join("Antigravity IDE.exe");
                    if c.is_file() {
                        return Some(c.to_string_lossy().to_string());
                    }
                }
            }
        }
        "factory" => {
            // 1. Check %LOCALAPPDATA%\Programs\Factory\Factory.exe and variants
            if !local_app_data.is_empty() {
                let factory_root = Path::new(&local_app_data).join("Factory");
                if let Some(found) = find_latest_squirrel_exe(&factory_root, "Factory.exe") {
                    return Some(found);
                }
                for sub in &[
                    r"Programs\Factory\Factory.exe",
                    r"Factory\Factory.exe",
                    r"Programs\Factory Desktop\Factory.exe",
                    r"Programs\FactoryAI\Factory.exe",
                ] {
                    let candidate = Path::new(&local_app_data).join(sub);
                    if candidate.is_file() {
                        return Some(candidate.to_string_lossy().to_string());
                    }
                }
            }

            // 2. Check ProgramFiles / ProgramFiles(x86)
            for base in &[&prog_files, &prog_files_x86] {
                if base.is_empty() {
                    continue;
                }
                for sub in &[
                    r"Factory\Factory.exe",
                    r"Factory Desktop\Factory.exe",
                    r"Programs\Factory\Factory.exe",
                ] {
                    let candidate = Path::new(base).join(sub);
                    if candidate.is_file() {
                        return Some(candidate.to_string_lossy().to_string());
                    }
                }
            }

            // 3. Check Uninstall registry
            if let Some(path) = search_uninstall_registry("factory", false, "Factory.exe") {
                return Some(path);
            }

            // 4. Check user bin directory for droid.exe
            if let Some(user_home) = dirs::home_dir() {
                for sub in &[
                    r"bin\droid.exe",
                    r".local\bin\droid.exe",
                    r".factory\bin\droid.exe",
                    r"AppData\Local\dsw\bin\dsw.exe",
                ] {
                    let candidate = user_home.join(sub);
                    if candidate.is_file() {
                        return Some(candidate.to_string_lossy().to_string());
                    }
                }
            }

            // 5. Check PATH for Factory.exe or droid.exe
            if let Ok(path_var) = std::env::var("PATH") {
                for dir in std::env::split_paths(&path_var) {
                    for bin_name in &["Factory.exe", "droid.exe"] {
                        let candidate = dir.join(bin_name);
                        if is_executable_or_alias(&candidate) {
                            return Some(candidate.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
        _ => {}
    }
    None
}

#[cfg(target_os = "windows")]
fn is_executable_or_alias(path: &Path) -> bool {
    path.is_file() || std::fs::symlink_metadata(path).map(|m| !m.is_dir()).unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn parse_msix_package_from_path(path_str: &str) -> Option<(PathBuf, String, String)> {
    let norm = path_str.replace('/', "\\");
    let norm_low = norm.to_lowercase();
    if !norm_low.contains("\\windowsapps\\") || norm_low.contains("\\microsoft\\windowsapps\\") {
        return None;
    }

    let path = Path::new(&norm);
    let mut pkg_root: Option<PathBuf> = None;
    let mut pkg_full_name: Option<String> = None;

    for ancestor in path.ancestors() {
        if let Some(parent) = ancestor.parent() {
            if parent
                .file_name()
                .and_then(|n| n.to_str())
                .map_or(false, |n| n.eq_ignore_ascii_case("WindowsApps"))
            {
                if let Some(folder_name) = ancestor.file_name().and_then(|n| n.to_str()) {
                    pkg_root = Some(ancestor.to_path_buf());
                    pkg_full_name = Some(folder_name.to_string());
                    break;
                }
            }
        }
    }

    let pkg_root = pkg_root?;
    let pkg_full_name = pkg_full_name?;

    // MSIX PackageFullName format: <Name>_<Version>_<Arch>_<ResourceId>_<PublisherId>
    let parts: Vec<&str> = pkg_full_name.rsplitn(5, '_').collect();
    let pkg_family_name = if parts.len() == 5 && !parts[4].is_empty() && !parts[0].is_empty() {
        format!("{}_{}", parts[4], parts[0])
    } else {
        let mut iter = pkg_full_name.split('_');
        let first = iter.next()?;
        let last = pkg_full_name.rsplit('_').next()?;
        if first.is_empty() || last.is_empty() || first == last {
            return None;
        }
        format!("{}_{}", first, last)
    };

    Some((pkg_root, pkg_full_name, pkg_family_name))
}

#[cfg(target_os = "windows")]
fn extract_xml_attr<'a>(tag: &'a str, attr_name: &str) -> Option<&'a str> {
    static ID_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static EXE_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();

    let re = match attr_name {
        name if name.eq_ignore_ascii_case("Id") => {
            ID_RE.get_or_init(|| regex::Regex::new(r#"(?i)\bId\s*=\s*"([^"]+)""#).expect("valid regex"))
        }
        name if name.eq_ignore_ascii_case("Executable") => EXE_RE.get_or_init(|| {
            regex::Regex::new(r#"(?i)\bExecutable\s*=\s*"([^"]+)""#).expect("valid regex")
        }),
        _ => return None,
    };
    re.captures(tag).and_then(|c| c.get(1)).map(|m| m.as_str())
}

#[cfg(target_os = "windows")]
fn manifest_xml_regexes() -> (&'static regex::Regex, &'static regex::Regex, &'static regex::Regex) {
    static APP_BLOCK_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static EXT_BLOCK_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static ALIAS_TAG_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();

    let app_re = APP_BLOCK_RE.get_or_init(|| {
        regex::Regex::new(r"(?is)<Application\b([^>]*)>(.*?)</Application>").expect("valid regex")
    });
    let ext_re = EXT_BLOCK_RE.get_or_init(|| {
        regex::Regex::new(
            r#"(?is)<(?:[a-zA-Z0-9_]+:)?Extension\b([^>]*Category\s*=\s*"windows\.appExecutionAlias"[^>]*)>(.*?)</(?:[a-zA-Z0-9_]+:)?Extension>"#,
        )
        .expect("valid regex")
    });
    let alias_re = ALIAS_TAG_RE.get_or_init(|| {
        regex::Regex::new(r#"(?is)<(?:[a-zA-Z0-9_]+:)?ExecutionAlias\b[^>]*\bAlias\s*=\s*"([^"]+)""#)
            .expect("valid regex")
    });

    (app_re, ext_re, alias_re)
}

#[cfg(target_os = "windows")]
fn parse_manifest_content_app_info(xml: &str, target_exe_name: &str) -> (Option<String>, Vec<String>) {
    let (app_re, ext_re, alias_re) = manifest_xml_regexes();

    let target_low = target_exe_name.to_lowercase();
    let mut matched_app_id: Option<String> = None;
    let mut matched_aliases: Vec<String> = Vec::new();
    let mut fallback_app_id: Option<String> = None;
    let mut fallback_aliases: Vec<String> = Vec::new();

    for caps in app_re.captures_iter(xml) {
        let app_attrs = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let app_body = caps.get(2).map(|m| m.as_str()).unwrap_or("");

        let app_id = extract_xml_attr(app_attrs, "Id").map(|s| s.to_string());
        let app_exe = extract_xml_attr(app_attrs, "Executable").unwrap_or("");
        let app_exe_fname = Path::new(&app_exe.replace('/', "\\"))
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();

        let is_target_app = !target_low.is_empty() && app_exe_fname == target_low;

        let mut app_aliases = Vec::new();
        for ext_caps in ext_re.captures_iter(app_body) {
            let ext_attrs = ext_caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let ext_body = ext_caps.get(2).map(|m| m.as_str()).unwrap_or("");

            if let Some(ext_exe) = extract_xml_attr(ext_attrs, "Executable") {
                let ext_exe_fname = Path::new(&ext_exe.replace('/', "\\"))
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if !target_low.is_empty() && !ext_exe_fname.is_empty() && ext_exe_fname != target_low {
                    continue;
                }
            }

            for al_caps in alias_re.captures_iter(ext_body) {
                if let Some(al) = al_caps.get(1) {
                    let alias_str = al.as_str().trim().to_string();
                    if !alias_str.is_empty() && !app_aliases.contains(&alias_str) {
                        app_aliases.push(alias_str);
                    }
                }
            }
        }

        if is_target_app && matched_app_id.is_none() {
            matched_app_id = app_id;
            matched_aliases = app_aliases;
            break;
        } else if fallback_app_id.is_none() {
            fallback_app_id = app_id;
            fallback_aliases = app_aliases;
        }
    }

    if matched_app_id.is_some() || !matched_aliases.is_empty() {
        (matched_app_id, matched_aliases)
    } else {
        (fallback_app_id, fallback_aliases)
    }
}

#[cfg(target_os = "windows")]
fn parse_manifest_app_info(pkg_root: &Path, target_exe_name: &str) -> (Option<String>, Vec<String>) {
    let manifest_path = pkg_root.join("AppxManifest.xml");
    if let Ok(xml) = std::fs::read_to_string(&manifest_path) {
        return parse_manifest_content_app_info(&xml, target_exe_name);
    }
    (None, Vec::new())
}

#[cfg(target_os = "windows")]
fn resolve_manifest_executable_in_xml(
    xml: &str,
    alias_or_exe_name: &str,
    allow_fallback: bool,
) -> Option<String> {
    let (app_re, ext_re, alias_re) = manifest_xml_regexes();

    let target_low = alias_or_exe_name.to_lowercase();
    let mut fallback_exe: Option<String> = None;

    for caps in app_re.captures_iter(xml) {
        let app_attrs = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let app_body = caps.get(2).map(|m| m.as_str()).unwrap_or("");

        let app_exe = extract_xml_attr(app_attrs, "Executable").unwrap_or("");
        if !app_exe.is_empty() && fallback_exe.is_none() {
            fallback_exe = Some(app_exe.to_string());
        }

        let app_exe_fname = Path::new(&app_exe.replace('/', "\\"))
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !target_low.is_empty() && app_exe_fname == target_low {
            return Some(app_exe.to_string());
        }

        for ext_caps in ext_re.captures_iter(app_body) {
            let ext_attrs = ext_caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let ext_body = ext_caps.get(2).map(|m| m.as_str()).unwrap_or("");
            let ext_exe = extract_xml_attr(ext_attrs, "Executable").unwrap_or(app_exe);

            for al_caps in alias_re.captures_iter(ext_body) {
                if let Some(al) = al_caps.get(1) {
                    if al.as_str().trim().eq_ignore_ascii_case(&target_low) && !ext_exe.is_empty() {
                        return Some(ext_exe.to_string());
                    }
                }
            }
        }
    }

    if allow_fallback {
        fallback_exe
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn find_appx_exe_by_family_or_alias(exe_path: &str) -> Option<String> {
    if exe_path.is_empty() {
        return None;
    }

    let norm = exe_path.replace('/', "\\");
    let p = Path::new(&norm);
    let exe_name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if exe_name.is_empty() {
        return None;
    }

    let mut family_hints: Vec<String> = Vec::new();
    if let Some(parent) = p.parent() {
        let parent_name = parent.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if parent_name.eq_ignore_ascii_case("WindowsApps") {
            // Global alias (%LOCALAPPDATA%\Microsoft\WindowsApps\<alias>.exe):
            // collect any package-scoped alias subdirectories containing the same alias name.
            if let Ok(entries) = std::fs::read_dir(parent) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let sub_path = entry.path();
                    if sub_path.is_dir() && is_executable_or_alias(&sub_path.join(exe_name)) {
                        family_hints.push(entry.file_name().to_string_lossy().to_lowercase());
                    }
                }
            }
        } else if parent
            .parent()
            .and_then(|gp| gp.file_name())
            .and_then(|n| n.to_str())
            .map_or(false, |n| n.eq_ignore_ascii_case("WindowsApps"))
        {
            // Package-scoped alias (%LOCALAPPDATA%\Microsoft\WindowsApps\<PackageFamilyName>\<alias>.exe)
            family_hints.push(parent_name.to_lowercase());
        }
    }

    let hkcu = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(
            r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages",
            KEY_READ,
        )
        .ok()?;

    let mut candidates: Vec<(bool, std::time::SystemTime, String)> = Vec::new();

    for pkg_name in hkcu.enum_keys().filter_map(|r| r.ok()) {
        let low = pkg_name.to_lowercase();
        let matched_by_family = family_hints.iter().any(|pfn_low| {
            if let Some((name_part, pub_part)) = pfn_low.rsplit_once('_') {
                low.starts_with(&format!("{}_", name_part))
                    && low.ends_with(&format!("_{}", pub_part))
            } else {
                false
            }
        });

        if let Ok(pkg_key) = hkcu.open_subkey_with_flags(&pkg_name, KEY_READ) {
            if let Ok(root) = pkg_key.get_value::<String, _>("PackageRootFolder") {
                let root_path = Path::new(&root);
                let mut resolved_candidate: Option<PathBuf> = None;

                if matched_by_family {
                    for direct in [root_path.join("app").join(exe_name), root_path.join(exe_name)] {
                        if direct.is_file() {
                            resolved_candidate = Some(direct);
                            break;
                        }
                    }
                }

                if resolved_candidate.is_none() {
                    let manifest_path = root_path.join("AppxManifest.xml");
                    if let Ok(xml) = std::fs::read_to_string(&manifest_path) {
                        if let Some(rel_exe) =
                            resolve_manifest_executable_in_xml(&xml, exe_name, matched_by_family)
                        {
                            let candidate = root_path.join(rel_exe.replace('/', "\\"));
                            if candidate.is_file() {
                                resolved_candidate = Some(candidate);
                            }
                        }
                    }
                }

                if let Some(candidate) = resolved_candidate {
                    let modified = candidate
                        .metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    candidates.push((
                        matched_by_family,
                        modified,
                        candidate.to_string_lossy().to_string(),
                    ));
                }
            }
        }
    }

    candidates.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| b.2.cmp(&a.2))
    });
    candidates.into_iter().next().map(|(_, _, path)| path)
}

#[cfg(target_os = "windows")]
fn is_helper_alias_name(fname_low: &str) -> bool {
    fname_low.contains("native-host")
        || fname_low.contains("command-runner")
        || fname_low.contains("elevated")
        || fname_low.contains("autostarter")
        || fname_low.contains("update")
        || fname_low.contains("crashpad")
}

#[cfg(target_os = "windows")]
fn is_installer_or_helper_exe(fname_low: &str) -> bool {
    fname_low == "update.exe"
        || fname_low == "squirrel.exe"
        || fname_low.starts_with("uninstall")
        || fname_low.starts_with("unins")
        || is_helper_alias_name(fname_low)
}

#[cfg(target_os = "windows")]
pub fn resolve_msix_execution_alias(path_str: &str) -> Option<String> {
    let local_app_data = std::env::var("LOCALAPPDATA").ok()?;
    if local_app_data.is_empty() {
        return None;
    }

    let (pkg_root, _pkg_full_name, pkg_family_name) = parse_msix_package_from_path(path_str)?;
    let target_exe_name = Path::new(path_str)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let target_low = target_exe_name.to_lowercase();

    let winapps_root = Path::new(&local_app_data).join("Microsoft").join("WindowsApps");
    let pfn_dir = winapps_root.join(&pkg_family_name);

    let (_app_id, manifest_aliases) = parse_manifest_app_info(&pkg_root, target_exe_name);

    // 1. Check aliases declared in AppxManifest.xml for this application
    for alias in &manifest_aliases {
        let global_alias = winapps_root.join(alias);
        if is_executable_or_alias(&global_alias) {
            return Some(global_alias.to_string_lossy().to_string());
        }
        let scoped_alias = pfn_dir.join(alias);
        if is_executable_or_alias(&scoped_alias) {
            return Some(scoped_alias.to_string_lossy().to_string());
        }
    }

    // 2. Check package-scoped alias directory (%LOCALAPPDATA%\Microsoft\WindowsApps\<PackageFamilyName>\)
    if pfn_dir.is_dir() {
        if !target_exe_name.is_empty() {
            let exact = pfn_dir.join(target_exe_name);
            if is_executable_or_alias(&exact) {
                let global = winapps_root.join(target_exe_name);
                if is_executable_or_alias(&global) {
                    return Some(global.to_string_lossy().to_string());
                }
                return Some(exact.to_string_lossy().to_string());
            }
        }

        if let Ok(entries) = std::fs::read_dir(&pfn_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let p = entry.path();
                let fname = entry.file_name().to_string_lossy().to_string();
                let fname_low = fname.to_lowercase();
                if fname_low.ends_with(".exe")
                    && !is_helper_alias_name(&fname_low)
                    && is_executable_or_alias(&p)
                {
                    let global = winapps_root.join(&fname);
                    if is_executable_or_alias(&global) {
                        return Some(global.to_string_lossy().to_string());
                    }
                    return Some(p.to_string_lossy().to_string());
                }
            }
        }
    }

    // 3. Fallback to global %LOCALAPPDATA%\Microsoft\WindowsApps aliases by family/target name
    let pfn_low = pkg_family_name.to_lowercase();
    let mut fallback_names: Vec<&str> = Vec::new();
    if !target_exe_name.is_empty() {
        fallback_names.push(target_exe_name);
    }
    if pfn_low.starts_with("openai.") || target_low == "chatgpt.exe" {
        fallback_names.extend_from_slice(&["ChatGPT.exe", "chatgpt-desktop.exe"]);
    } else if pfn_low.starts_with("claude_") || pfn_low.starts_with("anthropic.") || target_low == "claude.exe" {
        fallback_names.extend_from_slice(&["Claude.exe", "claude-desktop.exe"]);
    }

    for name in fallback_names {
        let candidate = winapps_root.join(name);
        if is_executable_or_alias(&candidate) {
            return Some(candidate.to_string_lossy().to_string());
        }
    }

    None
}

#[cfg(target_os = "windows")]
pub fn activate_msix_application(app_id: &str, exe_path: &str, args: &[String]) -> Result<u32, String> {
    use std::ffi::c_void;
    use windows::core::{GUID, HSTRING};

    let target_pkg_path = if parse_msix_package_from_path(exe_path).is_some() {
        Some(exe_path.to_string())
    } else {
        match app_id {
            "chatgpt" => find_latest_appx_exe(&["openai.chatgpt", "openai.codex"], "ChatGPT.exe"),
            "claude" => find_latest_appx_exe(&["claude_", "anthropic"], "Claude.exe"),
            _ => None,
        }
        .or_else(|| find_appx_exe_by_family_or_alias(exe_path))
    };

    let pkg_exe = target_pkg_path
        .ok_or_else(|| "Could not locate MSIX package information for activation".to_string())?;
    let (pkg_root, _pkg_full_name, pkg_family_name) = parse_msix_package_from_path(&pkg_exe)
        .ok_or_else(|| "Failed to parse MSIX PackageFamilyName".to_string())?;

    let target_exe_name = Path::new(&pkg_exe)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let (manifest_app_id, _) = parse_manifest_app_info(&pkg_root, target_exe_name);
    let entry_id = manifest_app_id.unwrap_or_else(|| "App".to_string());
    let aumid = format!("{}!{}", pkg_family_name, entry_id);

    let joined_args = args
        .iter()
        .map(|a| {
            if a.chars().any(char::is_whitespace) && !a.starts_with('"') {
                format!("\"{}\"", a)
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    #[repr(C)]
    struct IApplicationActivationManagerVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        activate_application: unsafe extern "system" fn(
            this: *mut c_void,
            app_user_model_id: *const u16,
            arguments: *const u16,
            options: u32,
            process_id: *mut u32,
        ) -> i32,
        activate_for_file: usize,
        activate_for_protocol: usize,
    }

    #[repr(C)]
    struct ComPtr {
        vtbl: *const IApplicationActivationManagerVtbl,
    }

    #[link(name = "ole32.dll", kind = "raw-dylib")]
    extern "system" {
        fn CoInitializeEx(pv_reserved: *const c_void, dw_co_init: u32) -> i32;
        fn CoUninitialize();
        fn CoCreateInstance(
            rclsid: *const GUID,
            p_unk_outer: *mut c_void,
            dw_cls_context: u32,
            riid: *const GUID,
            ppv: *mut *mut c_void,
        ) -> i32;
    }

    const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const CLSCTX_LOCAL_SERVER: u32 = 0x4;
    const CLSCTX_INPROC_SERVER: u32 = 0x1;
    const AO_NONE: u32 = 0;

    let clsid = GUID::from_u128(0x45ba127d_10a8_46ea_8ab7_56ea9078943c);
    let iid = GUID::from_u128(0x2e941141_7f97_4756_ba1d_9decde894a3d);

    let aumid_h = HSTRING::from(&aumid);
    let args_h = HSTRING::from(&joined_args);

    unsafe {
        let init_hr = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED);
        let mut raw_ptr: *mut c_void = std::ptr::null_mut();
        let hr = CoCreateInstance(
            &clsid,
            std::ptr::null_mut(),
            CLSCTX_LOCAL_SERVER | CLSCTX_INPROC_SERVER,
            &iid,
            &mut raw_ptr,
        );
        if hr < 0 || raw_ptr.is_null() {
            if init_hr >= 0 {
                CoUninitialize();
            }
            return Err(format!(
                "CoCreateInstance(ApplicationActivationManager) failed: HRESULT 0x{:08X}",
                hr as u32
            ));
        }

        let com_obj = raw_ptr as *mut ComPtr;
        let vtbl = &*(*com_obj).vtbl;
        let mut pid: u32 = 0;
        let act_hr = (vtbl.activate_application)(
            raw_ptr,
            aumid_h.as_ptr(),
            args_h.as_ptr(),
            AO_NONE,
            &mut pid,
        );
        (vtbl.release)(raw_ptr);
        if init_hr >= 0 {
            CoUninitialize();
        }

        if act_hr < 0 {
            return Err(format!(
                "ActivateApplication({}) failed: HRESULT 0x{:08X}",
                aumid, act_hr as u32
            ));
        }
        Ok(pid)
    }
}

#[cfg(target_os = "windows")]
pub fn shell_execute_application(
    exe_path: &str,
    args: &[String],
    working_dir: Option<&str>,
    env_vars: &[(String, String)],
    proxy_enabled: bool,
) -> Result<u32, String> {
    use std::ffi::c_void;
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Threading::GetProcessId;

    #[repr(C)]
    struct SHELLEXECUTEINFOW {
        cb_size: u32,
        f_mask: u32,
        hwnd: *mut c_void,
        lp_verb: *const u16,
        lp_file: *const u16,
        lp_parameters: *const u16,
        lp_directory: *const u16,
        n_show: i32,
        h_inst_app: *mut c_void,
        lp_id_list: *mut c_void,
        lp_class: *const u16,
        hkey_class: *mut c_void,
        dw_hot_key: u32,
        h_icon_or_monitor: *mut c_void,
        h_process: *mut c_void,
    }

    #[link(name = "shell32.dll", kind = "raw-dylib")]
    extern "system" {
        fn ShellExecuteExW(p_exec_info: *mut SHELLEXECUTEINFOW) -> i32;
    }

    const SEE_MASK_NOCLOSEPROCESS: u32 = 0x0000_0040;
    const SEE_MASK_NOASYNC: u32 = 0x0000_0100;
    const SW_SHOWNORMAL: i32 = 1;

    let joined_args = args
        .iter()
        .map(|a| {
            if a.chars().any(char::is_whitespace) && !a.starts_with('"') {
                format!("\"{}\"", a)
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    let file_h = HSTRING::from(exe_path);
    let params_h = HSTRING::from(&joined_args);
    let dir_h = working_dir.map(HSTRING::from);

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let saved_env: Vec<(&str, Option<std::ffi::OsString>)> = crate::core::proxy::PROXY_ENV_KEYS
        .iter()
        .map(|&k| (k, std::env::var_os(k)))
        .collect();

    struct EnvRestoreGuard(Vec<(&'static str, Option<std::ffi::OsString>)>);
    impl Drop for EnvRestoreGuard {
        fn drop(&mut self) {
            for (k, val) in &self.0 {
                match val {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }
    let _restore = EnvRestoreGuard(saved_env);

    if proxy_enabled {
        for (k, v) in env_vars {
            std::env::set_var(k, v);
        }
    } else {
        for &k in crate::core::proxy::PROXY_ENV_KEYS {
            std::env::remove_var(k);
        }
    }

    let mut sei = SHELLEXECUTEINFOW {
        cb_size: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        f_mask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        hwnd: std::ptr::null_mut(),
        lp_verb: std::ptr::null(),
        lp_file: file_h.as_ptr(),
        lp_parameters: if joined_args.is_empty() {
            std::ptr::null()
        } else {
            params_h.as_ptr()
        },
        lp_directory: dir_h.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
        n_show: SW_SHOWNORMAL,
        h_inst_app: std::ptr::null_mut(),
        lp_id_list: std::ptr::null_mut(),
        lp_class: std::ptr::null(),
        hkey_class: std::ptr::null_mut(),
        dw_hot_key: 0,
        h_icon_or_monitor: std::ptr::null_mut(),
        h_process: std::ptr::null_mut(),
    };

    unsafe {
        if ShellExecuteExW(&mut sei) == 0 {
            return Err(format!(
                "ShellExecuteExW failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        let mut pid: u32 = 0;
        if !sei.h_process.is_null() {
            let handle = HANDLE(sei.h_process);
            pid = GetProcessId(handle);
            let _ = CloseHandle(handle);
        }
        Ok(pid)
    }
}

#[cfg(target_os = "windows")]
fn find_latest_appx_exe(prefixes: &[&str], exe_name: &str) -> Option<String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(
            r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages",
            KEY_READ,
        )
        .ok()?;

    let mut candidates: Vec<(std::time::SystemTime, String)> = Vec::new();

    for pkg_name in hkcu.enum_keys().filter_map(|r| r.ok()) {
        let low = pkg_name.to_lowercase();
        if !prefixes.iter().any(|p| low.starts_with(p)) {
            continue;
        }
        if let Ok(pkg_key) = hkcu.open_subkey_with_flags(&pkg_name, KEY_READ) {
            if let Ok(root) = pkg_key.get_value::<String, _>("PackageRootFolder") {
                let root_path = Path::new(&root);
                for candidate in [root_path.join("app").join(exe_name), root_path.join(exe_name)] {
                    if candidate.is_file() {
                        let modified = candidate
                            .metadata()
                            .and_then(|m| m.modified())
                            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                        candidates.push((modified, candidate.to_string_lossy().to_string()));
                        break;
                    }
                }
            }
        }
    }

    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    candidates.into_iter().next().map(|(_, path)| path)
}

#[cfg(target_os = "windows")]
fn find_latest_squirrel_exe(root_dir: &Path, exe_name: &str) -> Option<String> {
    let entries = std::fs::read_dir(root_dir).ok()?;
    let mut candidates: Vec<(std::time::SystemTime, String)> = Vec::new();

    for entry in entries.filter_map(|e| e.ok()) {
        let file_name = entry.file_name().to_string_lossy().to_lowercase();
        if file_name.starts_with("app-") && entry.path().is_dir() {
            let exe_path = entry.path().join(exe_name);
            if exe_path.is_file() {
                let modified = exe_path
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                candidates.push((modified, exe_path.to_string_lossy().to_string()));
            }
        }
    }

    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    candidates.into_iter().next().map(|(_, path)| path)
}

#[cfg(target_os = "windows")]
fn find_latest_any_squirrel_exe(root_dir: &Path) -> Option<String> {
    let entries = std::fs::read_dir(root_dir).ok()?;
    let mut candidates: Vec<(std::time::SystemTime, String)> = Vec::new();

    for entry in entries.filter_map(|e| e.ok()) {
        let file_name = entry.file_name().to_string_lossy().to_lowercase();
        let app_dir = entry.path();
        if file_name.starts_with("app-") && app_dir.is_dir() {
            if let Ok(sub_entries) = std::fs::read_dir(&app_dir) {
                for sub in sub_entries.filter_map(|e| e.ok()) {
                    let exe_path = sub.path();
                    if exe_path.is_file()
                        && exe_path
                            .extension()
                            .map_or(false, |ext| ext.eq_ignore_ascii_case("exe"))
                    {
                        let fname_low = exe_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .to_lowercase();
                        if !is_installer_or_helper_exe(&fname_low) {
                            let modified = exe_path
                                .metadata()
                                .and_then(|m| m.modified())
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                            candidates.push((modified, exe_path.to_string_lossy().to_string()));
                        }
                    }
                }
            }
        }
    }

    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    candidates.into_iter().next().map(|(_, path)| path)
}

#[cfg(target_os = "windows")]
fn search_uninstall_registry(pattern: &str, require_ide: bool, fallback_exe: &str) -> Option<String> {
    let hives = [
        (HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_LOCAL_MACHINE, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_LOCAL_MACHINE, r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
    ];

    for (hive, subkey) in hives {
        if let Ok(root) = RegKey::predef(hive).open_subkey_with_flags(subkey, KEY_READ) {
            for item in root.enum_keys().filter_map(|r| r.ok()) {
                if let Ok(key) = root.open_subkey_with_flags(&item, KEY_READ) {
                    let disp_name = key.get_value::<String, _>("DisplayName").unwrap_or_default();
                    let disp_name_low = disp_name.to_lowercase();

                    if disp_name_low.contains(pattern) {
                        let is_ide = disp_name_low.contains("ide");
                        if require_ide != is_ide {
                            continue;
                        }

                        // Try InstallLocation first (including Squirrel app-* and app\ subdirectories)
                        if let Ok(loc) = key.get_value::<String, _>("InstallLocation") {
                            let cleaned = loc.trim().trim_matches('"');
                            if !cleaned.is_empty() {
                                let loc_path = Path::new(cleaned);
                                if let Some(squirrel_exe) = find_latest_squirrel_exe(loc_path, fallback_exe) {
                                    return Some(squirrel_exe);
                                }
                                let app_sub = loc_path.join("app").join(fallback_exe);
                                if app_sub.is_file() {
                                    return Some(app_sub.to_string_lossy().to_string());
                                }
                                let candidate = loc_path.join(fallback_exe);
                                if candidate.is_file() {
                                    return Some(candidate.to_string_lossy().to_string());
                                }
                            }
                        }

                        // Try DisplayIcon (excluding Squirrel Update.exe, uninstallers, and helpers)
                        if let Ok(icon) = key.get_value::<String, _>("DisplayIcon") {
                            let cleaned = icon.split(',').next().unwrap_or("").trim().trim_matches('"');
                            let icon_path = Path::new(cleaned);
                            if let Some(fname) = icon_path.file_name().and_then(|n| n.to_str()) {
                                let fname_low = fname.to_lowercase();
                                if icon_path.is_file()
                                    && fname_low.ends_with(".exe")
                                    && !is_installer_or_helper_exe(&fname_low)
                                {
                                    return Some(cleaned.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
pub fn request_graceful_close(pids: &[u32]) -> bool {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetAncestor, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, GA_ROOT, WM_CLOSE,
    };

    struct Context<'a> {
        pids: &'a [u32],
        closed_any: bool,
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut Context);
        let mut proc_id: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut proc_id));
        if ctx.pids.contains(&proc_id) {
            let is_visible = IsWindowVisible(hwnd).as_bool();
            let is_root = GetAncestor(hwnd, GA_ROOT) == hwnd;
            if is_visible || is_root {
                let _ = PostMessageW(hwnd, WM_CLOSE, None, None);
                ctx.closed_any = true;
            }
        }
        BOOL(1)
    }

    let mut ctx = Context {
        pids,
        closed_any: false,
    };

    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
    }
    ctx.closed_any
}

#[cfg(target_os = "windows")]
pub fn get_process_tcp_connections(pids: &std::collections::HashSet<u32>, proxy_port: u16) -> Vec<String> {
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_ALL,
    };
    use windows::Win32::Networking::WinSock::AF_INET;

    let mut records = Vec::new();
    let mut size: u32 = 0;

    unsafe {
        // Query initial table size
        let _ = GetExtendedTcpTable(
            None,
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );

        if size == 0 {
            return records;
        }

        // Allocate buffer with headroom to avoid race conditions during socket bursts
        let mut buffer: Vec<u8> = vec![0; ((size as usize * 12) / 10) + 256];
        let mut alloc_size = buffer.len() as u32;

        let mut ret = GetExtendedTcpTable(
            Some(buffer.as_mut_ptr() as *mut _),
            &mut alloc_size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );

        // Retry once if buffer was still insufficient
        if ret == 122 {
            buffer = vec![0; (alloc_size as usize) + 512];
            alloc_size = buffer.len() as u32;
            ret = GetExtendedTcpTable(
                Some(buffer.as_mut_ptr() as *mut _),
                &mut alloc_size,
                false,
                AF_INET.0 as u32,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            );
        }

        if ret == 0 {
            let table = &*(buffer.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
            let entries = std::slice::from_raw_parts(
                table.table.as_ptr(),
                table.dwNumEntries as usize,
            );

            for row in entries {
                let owning_pid = row.dwOwningPid;
                if pids.contains(&owning_pid) {
                    let remote_port = u16::from_be((row.dwRemotePort & 0xFFFF) as u16);
                    let remote_ip = std::net::Ipv4Addr::from(row.dwRemoteAddr.to_ne_bytes());
                    let local_port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);

                    let state_str = match row.dwState {
                        1 => "CLOSED",
                        2 => "LISTEN",
                        3 => "SYN_SENT",
                        4 => "SYN_RCVD",
                        5 => "ESTABLISHED",
                        6 => "FIN_WAIT1",
                        7 => "FIN_WAIT2",
                        8 => "CLOSE_WAIT",
                        9 => "CLOSING",
                        10 => "LAST_ACK",
                        11 => "TIME_WAIT",
                        12 => "DELETE_TCB",
                        _ => "UNKNOWN",
                    };

                    if remote_port == proxy_port || state_str == "ESTABLISHED" {
                        let is_proxy = if remote_port == proxy_port { " [指向代理端口]" } else { "" };
                        records.push(format!(
                            "✔ PID {} (端口 :{}) -> {}:{}: {}{}",
                            owning_pid, local_port, remote_ip, remote_port, state_str, is_proxy
                        ));
                    }
                }
            }
        }
    }
    records
}

#[cfg(target_os = "windows")]
fn resolve_shortcut_icon_exe(app_id: &str, target_icon_exe: &str) -> Option<String> {
    let norm_low = target_icon_exe.replace('/', "\\").to_lowercase();
    if !target_icon_exe.is_empty()
        && !norm_low.contains("\\microsoft\\windowsapps\\")
        && Path::new(target_icon_exe).is_file()
    {
        return Some(target_icon_exe.to_string());
    }

    match app_id {
        "chatgpt" => find_latest_appx_exe(&["openai.chatgpt", "openai.codex"], "ChatGPT.exe"),
        "claude" => find_latest_appx_exe(&["claude_", "anthropic"], "Claude.exe"),
        _ => None,
    }
    .or_else(|| find_appx_exe_by_family_or_alias(target_icon_exe))
}

#[cfg(target_os = "windows")]
pub fn create_desktop_shortcut(app_id: &str, app_name: &str, target_icon_exe: &str) -> Result<String, String> {
    let desktop = dirs::desktop_dir().ok_or_else(|| "Could not find desktop directory".to_string())?;

    // Filter invalid Windows filename characters to avoid OS error 123
    let clean_app_name: String = app_name
        .chars()
        .map(|c| match c {
            ':' | '*' | '?' | '"' | '<' | '>' | '|' | '/' | '\\' => '_',
            other => other,
        })
        .collect();

    let link_path = desktop.join(format!("{} (Local-AI-Route).lnk", clean_app_name));

    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;

    let mut sl = mslnk::ShellLink::new(&current_exe).map_err(|e| e.to_string())?;
    sl.set_arguments(Some(format!("--launch {}", app_id)));
    if let Some(parent) = current_exe.parent() {
        sl.set_working_dir(Some(parent.to_string_lossy().to_string()));
    }
    if let Some(icon_exe) = resolve_shortcut_icon_exe(app_id, target_icon_exe) {
        sl.set_icon_location(Some(icon_exe));
    }
    sl.create_lnk(&link_path).map_err(|e| e.to_string())?;

    Ok(link_path.to_string_lossy().to_string())
}

#[cfg(target_os = "windows")]
pub fn show_native_message_box(title: &str, message: &str) {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONWARNING, MB_OK};

    let title_h = HSTRING::from(title);
    let message_h = HSTRING::from(message);
    unsafe {
        let _ = MessageBoxW(None, &message_h, &title_h, MB_OK | MB_ICONWARNING);
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    #[test]
    fn test_parse_msix_package_from_path() {
        let p = r"C:\Program Files\WindowsApps\OpenAI.Codex_26.1027.3384.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe";
        let res = parse_msix_package_from_path(p);
        assert!(res.is_some());
        let (root, full, family) = res.unwrap();
        assert_eq!(
            root,
            PathBuf::from(r"C:\Program Files\WindowsApps\OpenAI.Codex_26.1027.3384.0_x64__2p2nqsd0c76g0")
        );
        assert_eq!(full, "OpenAI.Codex_26.1027.3384.0_x64__2p2nqsd0c76g0");
        assert_eq!(family, "OpenAI.Codex_2p2nqsd0c76g0");

        let alias = r"C:\Users\apuser\AppData\Local\Microsoft\WindowsApps\chatgpt-desktop.exe";
        assert!(parse_msix_package_from_path(alias).is_none());
    }

    #[test]
    fn test_parse_manifest_content_app_info() {
        let sample_xml = r#"
        <Applications>
          <Application Id="App" Executable="app/ChatGPT.exe" EntryPoint="Windows.FullTrustApplication">
            <Extensions>
              <uap3:Extension Category="windows.appExecutionAlias">
                <uap3:AppExecutionAlias>
                  <desktop:ExecutionAlias Alias="chatgpt-desktop.exe" />
                </uap3:AppExecutionAlias>
              </uap3:Extension>
            </Extensions>
          </Application>
          <Application Id="CodexCoreCommandRunner" Executable="app/resources/codex-command-runner.exe" EntryPoint="Windows.FullTrustApplication">
            <Extensions>
              <uap3:Extension Category="windows.appExecutionAlias" Executable="app/resources/codex-command-runner.exe">
                <uap3:AppExecutionAlias>
                  <desktop:ExecutionAlias Alias="codex-core-command-runner.exe" />
                </uap3:AppExecutionAlias>
              </uap3:Extension>
            </Extensions>
          </Application>
        </Applications>
        "#;
        let (app_id, aliases) = parse_manifest_content_app_info(sample_xml, "ChatGPT.exe");
        assert_eq!(app_id, Some("App".to_string()));
        assert_eq!(aliases, vec!["chatgpt-desktop.exe".to_string()]);

        let resolved_exe =
            resolve_manifest_executable_in_xml(sample_xml, "chatgpt-desktop.exe", false);
        assert_eq!(resolved_exe, Some("app/ChatGPT.exe".to_string()));
    }
}
