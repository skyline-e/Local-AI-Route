//! Client-side Antigravity location-restriction patch.
//!
//! Ported from the MIT-licensed signatures in
//! https://github.com/QNIX-Dev/eligibility-antigravity-patcher
//! The change only redirects a local client gate. It does not alter server
//! authentication, entitlements, or paid features.

use regex::bytes::Regex;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const BACKUP_SUFFIX: &str = ".larbak";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchStatus {
    Unpatched,
    Patched,
    Unknown,
    Missing,
    Locked,
}

impl PatchStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unpatched => "unpatched",
            Self::Patched => "patched",
            Self::Unknown => "unknown",
            Self::Missing => "missing",
            Self::Locked => "locked",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EligibilityStatus {
    pub app_id: String,
    pub status: PatchStatus,
    pub target_path: String,
    pub backup_path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EligibilityResult {
    pub success: bool,
    pub status: EligibilityStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arch {
    X64,
    Arm64,
}

struct BinaryGate {
    arch: Arch,
    original: Regex,
    patched: Regex,
    fix: &'static [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SliceInfo {
    arch: Option<Arch>,
    ranges: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MatchSite {
    offset: usize,
    gate_index: usize,
    patched: bool,
}

fn msg(is_en: bool, zh: &str, en: &str) -> String {
    if is_en {
        en.to_string()
    } else {
        zh.to_string()
    }
}

pub fn inspect(app_id: &str, exe_path: &str, is_en: bool) -> EligibilityStatus {
    match resolve_target(app_id, exe_path, is_en) {
        Ok(target) => inspect_target(app_id, &target, is_en),
        Err(message) => EligibilityStatus {
            app_id: app_id.to_string(),
            status: if exe_path.trim().is_empty() {
                PatchStatus::Missing
            } else {
                PatchStatus::Unknown
            },
            target_path: String::new(),
            backup_path: String::new(),
            message,
        },
    }
}

pub fn apply(app_id: &str, exe_path: &str, is_en: bool) -> Result<EligibilityResult, String> {
    let target = resolve_target(app_id, exe_path, is_en)?;
    if is_write_locked(&target) {
        return Ok(EligibilityResult {
            success: false,
            status: locked_status(app_id, &target, is_en),
        });
    }

    match app_id {
        "antigravity-app" => apply_binary(&target, app_id, is_en),
        "antigravity-ide" => apply_ide(&target, app_id, is_en),
        _ => Err(msg(
            is_en,
            "地区限制补丁仅支持 Antigravity 桌面版与 Antigravity IDE",
            "Location-restriction patch only supports Antigravity Desktop and Antigravity IDE.",
        )),
    }
}

pub fn restore(app_id: &str, exe_path: &str, is_en: bool) -> Result<EligibilityResult, String> {
    let target = resolve_target(app_id, exe_path, is_en)?;
    let backup = backup_path(&target);
    if !backup.is_file() {
        return Ok(EligibilityResult {
            success: false,
            status: EligibilityStatus {
                app_id: app_id.to_string(),
                status: inspect_file(app_id, &target, is_en).0,
                target_path: target.display().to_string(),
                backup_path: String::new(),
                message: msg(
                    is_en,
                    "没有可还原的备份。更新应用后需要重新解除限制。",
                    "No backup available to restore. Reapply the patch after updating the app.",
                ),
            },
        });
    }
    if is_write_locked(&target) {
        return Ok(EligibilityResult {
            success: false,
            status: locked_status(app_id, &target, is_en),
        });
    }

    let backup_bytes = fs::read(&backup).map_err(|e| {
        if is_en {
            format!("Failed to read backup: {e}")
        } else {
            format!("读取备份失败: {e}")
        }
    })?;
    let (backup_status, _) = inspect_bytes(app_id, &backup_bytes, Some(&backup), is_en)?;
    if backup_status != PatchStatus::Unpatched {
        return Err(msg(
            is_en,
            "备份不是已识别的原始文件，已拒绝还原。",
            "Backup is not a recognized original file; restore refused.",
        ));
    }

    write_verified(&target, &backup_bytes, is_en)?;
    let after = inspect_target(app_id, &target, is_en);
    if after.status != PatchStatus::Unpatched {
        return Err(msg(
            is_en,
            "还原后校验失败，请手动用备份覆盖目标文件。",
            "Post-restore verification failed. Please overwrite the target file with the backup manually.",
        ));
    }
    if app_id == "antigravity-ide" {
        clear_ide_bytecode_cache();
    }
    Ok(EligibilityResult {
        success: true,
        status: EligibilityStatus {
            message: msg(
                is_en,
                "已从备份还原原始文件。",
                "Original file restored from backup.",
            ),
            ..after
        },
    })
}

fn apply_binary(target: &Path, app_id: &str, is_en: bool) -> Result<EligibilityResult, String> {
    let original = fs::read(target).map_err(|e| {
        if is_en {
            format!("Failed to read language_server: {e}")
        } else {
            format!("读取 language_server 失败: {e}")
        }
    })?;
    let slices = executable_info(&original, Some(target), is_en)?;
    let (status, sites) = inspect_binary(&original, &slices, is_en)?;
    if status == PatchStatus::Patched {
        return Ok(already(app_id, target, is_en));
    }
    if sites.is_empty() {
        return Err(msg(
            is_en,
            "未找到可补丁的签名",
            "No patchable signature found.",
        ));
    }

    let backup = ensure_backup(target, &original, is_en)?;
    let mut patched = original.clone();
    let gates = manager_gates();
    for site in &sites {
        let gate = &gates[site.gate_index];
        let end = site
            .offset
            .checked_add(gate.fix.len())
            .ok_or_else(|| msg(is_en, "补丁写入范围溢出", "Patch write range overflow."))?;
        if end > patched.len() {
            return Err(msg(
                is_en,
                "补丁写入范围超出文件",
                "Patch write range exceeds file size.",
            ));
        }
        patched[site.offset..end].copy_from_slice(gate.fix);
    }

    if let Err(err) = write_verified(target, &patched, is_en) {
        let _ = restore_bytes(target, &backup, is_en);
        return Err(if is_en {
            format!("Failed to write patch (rollback attempted): {err}")
        } else {
            format!("写入补丁失败，已尝试回滚: {err}")
        });
    }

    let pre_sign_ok = matches!(
        inspect_binary_file(target, is_en),
        Ok((PatchStatus::Patched, verified))
            if verified.len() == sites.len()
                && verified
                    .iter()
                    .zip(sites.iter())
                    .all(|(v, s)| v.offset == s.offset && v.gate_index == s.gate_index)
    );
    if !pre_sign_ok {
        let _ = restore_bytes(target, &backup, is_en);
        return Err(msg(
            is_en,
            "补丁写入后校验失败，已回滚。",
            "Post-write verification failed; changes were rolled back.",
        ));
    }

    if let Err(err) = resign_macho_if_needed(target, &patched, is_en) {
        let _ = restore_bytes(target, &backup, is_en);
        return Err(err);
    }

    // Re-signing a Universal Mach-O binary can shift subsequent slice offsets,
    // so post-sign verification checks gate indices rather than raw file offsets.
    match inspect_binary_file(target, is_en) {
        Ok((PatchStatus::Patched, verified))
            if verified.len() == sites.len()
                && verified
                    .iter()
                    .zip(sites.iter())
                    .all(|(v, s)| v.gate_index == s.gate_index) =>
        {
            Ok(EligibilityResult {
                success: true,
                status: EligibilityStatus {
                    app_id: app_id.to_string(),
                    status: PatchStatus::Patched,
                    target_path: target.display().to_string(),
                    backup_path: backup_path(target).display().to_string(),
                    message: msg(
                        is_en,
                        "已解除桌面版本地地区限制。请重新启动 Antigravity。模型是否可用仍由服务端决定。",
                        "Desktop local location restriction removed. Please restart Antigravity. Model availability is still decided by the server.",
                    ),
                },
            })
        }
        _ => {
            let _ = restore_bytes(target, &backup, is_en);
            Err(msg(
                is_en,
                "补丁写入后校验失败，已回滚。",
                "Post-write verification failed; changes were rolled back.",
            ))
        }
    }
}

fn apply_ide(target: &Path, app_id: &str, is_en: bool) -> Result<EligibilityResult, String> {
    let original = fs::read(target).map_err(|e| {
        if is_en {
            format!("Failed to read main.js: {e}")
        } else {
            format!("读取 main.js 失败: {e}")
        }
    })?;
    let (status, _) = inspect_ide(&original, is_en)?;
    if status == PatchStatus::Patched {
        return Ok(already(app_id, target, is_en));
    }
    let patched = replace_ide_gate(&original, is_en)?;
    let backup = ensure_backup(target, &original, is_en)?;
    if let Err(err) = write_verified(target, &patched, is_en) {
        let _ = restore_bytes(target, &backup, is_en);
        return Err(if is_en {
            format!("Failed to write IDE patch (rollback attempted): {err}")
        } else {
            format!("写入 IDE 补丁失败，已尝试回滚: {err}")
        });
    }
    match inspect_ide_file(target, is_en) {
        Ok(PatchStatus::Patched) => {
            clear_ide_bytecode_cache();
            Ok(EligibilityResult {
                success: true,
                status: EligibilityStatus {
                    app_id: app_id.to_string(),
                    status: PatchStatus::Patched,
                    target_path: target.display().to_string(),
                    backup_path: backup_path(target).display().to_string(),
                    message: msg(
                        is_en,
                        "已解除 IDE 本地地区限制，并清理了字节码缓存。请重新启动 Antigravity IDE。",
                        "IDE local location restriction removed and bytecode cache cleared. Please restart Antigravity IDE.",
                    ),
                },
            })
        }
        _ => {
            let _ = restore_bytes(target, &backup, is_en);
            Err(msg(
                is_en,
                "IDE 补丁写入后校验失败，已回滚。",
                "IDE post-write verification failed; changes were rolled back.",
            ))
        }
    }
}

fn already(app_id: &str, target: &Path, is_en: bool) -> EligibilityResult {
    EligibilityResult {
        success: true,
        status: EligibilityStatus {
            app_id: app_id.to_string(),
            status: PatchStatus::Patched,
            target_path: target.display().to_string(),
            backup_path: existing_backup(target),
            message: msg(
                is_en,
                "本地地区限制已经解除，无需重复修改。",
                "Local location restriction is already removed; no changes needed.",
            ),
        },
    }
}

fn inspect_target(app_id: &str, target: &Path, is_en: bool) -> EligibilityStatus {
    let (status, message) = inspect_file(app_id, target, is_en);
    EligibilityStatus {
        app_id: app_id.to_string(),
        status,
        target_path: target.display().to_string(),
        backup_path: existing_backup(target),
        message,
    }
}

fn inspect_file(app_id: &str, target: &Path, is_en: bool) -> (PatchStatus, String) {
    match app_id {
        "antigravity-app" => match inspect_binary_file(target, is_en) {
            Ok((status, _)) => (status, status_message(app_id, status, is_en)),
            Err(err) => (PatchStatus::Unknown, err),
        },
        "antigravity-ide" => match inspect_ide_file(target, is_en) {
            Ok(status) => (status, status_message(app_id, status, is_en)),
            Err(err) => (PatchStatus::Unknown, err),
        },
        _ => (
            PatchStatus::Unknown,
            msg(is_en, "不支持的应用", "Unsupported application."),
        ),
    }
}

fn status_message(app_id: &str, status: PatchStatus, is_en: bool) -> String {
    match (app_id, status) {
        ("antigravity-app", PatchStatus::Patched) => msg(
            is_en,
            "桌面版本地地区限制已解除。应用更新后需要重新操作。",
            "Desktop local location restriction is removed. Reapply after updating the app.",
        ),
        ("antigravity-app", PatchStatus::Unpatched) => msg(
            is_en,
            "检测到桌面版本地地区限制。解除后需重启应用；模型可用性仍由服务端决定。",
            "Desktop local location restriction detected. Restart after removal; model availability is still decided by the server.",
        ),
        ("antigravity-ide", PatchStatus::Patched) => msg(
            is_en,
            "IDE 本地地区限制已解除。应用更新后需要重新操作。",
            "IDE local location restriction is removed. Reapply after updating the app.",
        ),
        ("antigravity-ide", PatchStatus::Unpatched) => msg(
            is_en,
            "检测到 IDE 本地地区限制。解除后会清理字节码缓存并需要重启。",
            "IDE local location restriction detected. Bytecode cache will be cleared on removal and a restart is required.",
        ),
        (_, PatchStatus::Unknown) => msg(
            is_en,
            "未识别当前版本的本地限制签名，文件未修改。请先更新到最新版本。",
            "Unrecognized local restriction signature for this version; file was not modified.",
        ),
        (_, PatchStatus::Missing) => msg(
            is_en,
            "未找到可修改的安装文件。",
            "Target installation file not found.",
        ),
        (_, PatchStatus::Locked) => msg(
            is_en,
            "目标文件正在使用，请先完全退出应用后再试。",
            "Target file is currently in use. Quit the app completely and try again.",
        ),
        _ => String::new(),
    }
}

fn locked_status(app_id: &str, target: &Path, is_en: bool) -> EligibilityStatus {
    EligibilityStatus {
        app_id: app_id.to_string(),
        status: PatchStatus::Locked,
        target_path: target.display().to_string(),
        backup_path: existing_backup(target),
        message: msg(
            is_en,
            "目标文件正在使用或没有写入权限。请先完全退出 Antigravity 后再试。",
            "Target file is in use or not writable. Quit Antigravity completely and try again.",
        ),
    }
}

fn resolve_target(app_id: &str, exe_path: &str, is_en: bool) -> Result<PathBuf, String> {
    let exe = exe_path.trim();
    if exe.is_empty() {
        return Err(msg(
            is_en,
            "未配置应用路径，请先自动检测或手动指定。",
            "Application path is not configured. Please auto-detect or specify it manually.",
        ));
    }
    let exe_path = PathBuf::from(exe);
    if !exe_path.is_file() {
        return Err(if is_en {
            format!("Application path does not exist: {}", exe_path.display())
        } else {
            format!("应用路径不存在: {}", exe_path.display())
        });
    }
    let install_root = install_root(&exe_path);
    let target = match app_id {
        "antigravity-app" => find_language_server(&install_root, is_en)?,
        "antigravity-ide" => find_main_js(&install_root, is_en)?,
        _ => {
            return Err(msg(
                is_en,
                "地区限制补丁仅支持 Antigravity 桌面版与 Antigravity IDE",
                "Location-restriction patch only supports Antigravity Desktop and Antigravity IDE.",
            ))
        }
    };
    Ok(target)
}

fn install_root(exe: &Path) -> PathBuf {
    let mut cursor = exe.to_path_buf();
    if cursor.is_file() {
        if let Some(parent) = cursor.parent() {
            cursor = parent.to_path_buf();
        }
    }
    // macOS: .../App.app/Contents/MacOS/Executable -> App.app
    if cursor.file_name().and_then(|n| n.to_str()) == Some("MacOS") {
        if let Some(contents) = cursor.parent() {
            if contents.file_name().and_then(|n| n.to_str()) == Some("Contents") {
                if let Some(bundle) = contents.parent() {
                    return bundle.to_path_buf();
                }
            }
        }
    }
    cursor
}

fn find_language_server(root: &Path, is_en: bool) -> Result<PathBuf, String> {
    let names = [
        "language_server.exe",
        "language_server_windows_x64.exe",
        "language_server_windows_arm.exe",
        "language_server",
    ];
    let mut candidates = Vec::new();
    for rel in ["resources/bin", "Contents/Resources/bin", "bin"] {
        let dir = root.join(rel);
        for name in names {
            let path = dir.join(name);
            if path.is_file() {
                candidates.push(path);
            }
        }
    }
    if candidates.is_empty() {
        return Err(msg(
            is_en,
            "未在安装目录中找到 language_server。请确认这是 Antigravity 桌面版。",
            "Could not find language_server in the installation directory. Please verify this is Antigravity Desktop.",
        ));
    }
    candidates.sort_by_key(|path| {
        std::cmp::Reverse(fs::metadata(path).ok().and_then(|m| m.modified().ok()))
    });
    Ok(candidates.remove(0))
}

fn find_main_js(root: &Path, is_en: bool) -> Result<PathBuf, String> {
    for rel in [
        "resources/app/out/main.js",
        "Contents/Resources/app/out/main.js",
    ] {
        let path = root.join(rel);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(msg(
        is_en,
        "未在安装目录中找到 resources/app/out/main.js。请确认这是 Antigravity IDE。",
        "Could not find resources/app/out/main.js in the installation directory. Please verify this is Antigravity IDE.",
    ))
}

fn inspect_binary_file(path: &Path, is_en: bool) -> Result<(PatchStatus, Vec<MatchSite>), String> {
    let data = fs::read(path).map_err(|e| {
        if is_en {
            format!("Failed to read binary: {e}")
        } else {
            format!("读取二进制失败: {e}")
        }
    })?;
    let slices = executable_info(&data, Some(path), is_en)?;
    inspect_binary(&data, &slices, is_en)
}

fn inspect_binary(
    data: &[u8],
    slices: &[SliceInfo],
    is_en: bool,
) -> Result<(PatchStatus, Vec<MatchSite>), String> {
    let gates = manager_gates();
    let mut sites = Vec::new();

    for slice in slices {
        let mut slice_found: Option<MatchSite> = None;
        for (gate_index, gate) in gates.iter().enumerate() {
            if slice.arch.is_some_and(|expected| expected != gate.arch) {
                continue;
            }
            for (patched, pattern) in [(false, &gate.original), (true, &gate.patched)] {
                if let Some(offset) = unique_match(pattern, data, &slice.ranges, is_en)? {
                    if slice_found.is_some() {
                        return Err(msg(
                            is_en,
                            "签名不唯一，已拒绝修改。",
                            "Signature is not unique; modification refused.",
                        ));
                    }
                    slice_found = Some(MatchSite {
                        offset,
                        gate_index,
                        patched,
                    });
                }
            }
        }
        if let Some(site) = slice_found {
            sites.push(site);
        }
    }

    if sites.is_empty() {
        return Err(msg(
            is_en,
            "未找到桌面版地区限制签名（可能是不支持的版本）。文件未修改。",
            "Desktop location-restriction signature not found (possibly an unsupported version). File was not modified.",
        ));
    }

    let all_patched = sites.iter().all(|s| s.patched);
    let all_unpatched = sites.iter().all(|s| !s.patched);
    if all_patched {
        Ok((PatchStatus::Patched, sites))
    } else if all_unpatched {
        Ok((PatchStatus::Unpatched, sites))
    } else {
        Err(msg(
            is_en,
            "多架构二进制中存在不一致的补丁状态，已拒绝修改。",
            "Inconsistent patch state across binary slices; modification refused.",
        ))
    }
}

fn inspect_ide_file(path: &Path, is_en: bool) -> Result<PatchStatus, String> {
    let data = fs::read(path).map_err(|e| {
        if is_en {
            format!("Failed to read main.js: {e}")
        } else {
            format!("读取 main.js 失败: {e}")
        }
    })?;
    inspect_ide(&data, is_en).map(|(status, _)| status)
}

fn inspect_ide(data: &[u8], is_en: bool) -> Result<(PatchStatus, Option<usize>), String> {
    let original = ide_original_re();
    let patched = ide_patched_re();
    let original_hit = unique_regex_match(original, data, is_en)?;
    let patched_hit = unique_regex_match(patched, data, is_en)?;
    match (original_hit, patched_hit) {
        (Some(_), Some(_)) => Err(msg(
            is_en,
            "IDE 同时存在原始和已修改签名，已拒绝修改。",
            "Both original and patched IDE signatures were found; modification refused.",
        )),
        (Some(offset), None) => Ok((PatchStatus::Unpatched, Some(offset))),
        (None, Some(_)) => Ok((PatchStatus::Patched, None)),
        (None, None) => Err(msg(
            is_en,
            "未找到 IDE 地区限制签名（可能是不支持的版本）。文件未修改。",
            "IDE location-restriction signature not found (possibly an unsupported version). File was not modified.",
        )),
    }
}

fn inspect_bytes(
    app_id: &str,
    data: &[u8],
    path_for_binary: Option<&Path>,
    is_en: bool,
) -> Result<(PatchStatus, ()), String> {
    match app_id {
        "antigravity-app" => {
            let slices = executable_info(data, path_for_binary, is_en)?;
            inspect_binary(data, &slices, is_en).map(|(status, _)| (status, ()))
        }
        "antigravity-ide" => inspect_ide(data, is_en).map(|(status, _)| (status, ())),
        _ => Err(msg(is_en, "不支持的应用", "Unsupported application.")),
    }
}

fn replace_ide_gate(data: &[u8], is_en: bool) -> Result<Vec<u8>, String> {
    let re = ide_original_re();
    let mut hits = re.find_iter(data);
    let Some(hit) = hits.next() else {
        return Err(msg(
            is_en,
            "未找到 IDE 地区限制签名。",
            "IDE location-restriction signature not found.",
        ));
    };
    if hits.next().is_some() {
        return Err(msg(
            is_en,
            "IDE 签名不唯一，已拒绝修改。",
            "IDE signature is not unique; modification refused.",
        ));
    }
    let caps = re
        .captures(data)
        .ok_or_else(|| msg(is_en, "IDE 签名解析失败", "Failed to parse IDE signature."))?;
    let prefix = caps
        .get(1)
        .ok_or_else(|| msg(is_en, "IDE 签名缺少前缀", "IDE signature is missing prefix."))?;
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..hit.start()]);
    out.extend_from_slice(prefix.as_bytes());
    out.extend_from_slice(b"true");
    out.extend_from_slice(&data[hit.end()..]);
    Ok(out)
}

fn unique_match(
    pattern: &Regex,
    data: &[u8],
    ranges: &[(usize, usize)],
    is_en: bool,
) -> Result<Option<usize>, String> {
    let mut found = None;
    for &(start, end) in ranges {
        let end = end.min(data.len());
        if start >= end {
            continue;
        }
        for hit in pattern.find_iter(&data[start..end]) {
            let offset = start + hit.start();
            if found.is_some() {
                return Err(msg(
                    is_en,
                    "签名不唯一，已拒绝修改。",
                    "Signature is not unique; modification refused.",
                ));
            }
            found = Some(offset);
        }
    }
    Ok(found)
}

fn unique_regex_match(pattern: &Regex, data: &[u8], is_en: bool) -> Result<Option<usize>, String> {
    let mut hits = pattern.find_iter(data);
    let first = hits.next().map(|hit| hit.start());
    if hits.next().is_some() {
        return Err(msg(
            is_en,
            "签名不唯一，已拒绝修改。",
            "Signature is not unique; modification refused.",
        ));
    }
    Ok(first)
}

fn manager_gates() -> &'static [BinaryGate] {
    static GATES: OnceLock<Vec<BinaryGate>> = OnceLock::new();
    GATES.get_or_init(|| {
        vec![
            BinaryGate {
                arch: Arch::X64,
                // cmp byte ptr [rax+8],0 ; je rel8 ; mov rdx,[rsp+0x70] ; mov [rax+0x60],rdx
                original: Regex::new(
                    r"(?-u)\x80\x78\x08\x00\x74.\x48\x8b\x54\x24\x70\x48\x89\x50\x60",
                )
                .expect("x64 original signature"),
                patched: Regex::new(
                    r"(?-u)\xc6\x40\x08\x01\x90\x90\x48\x8b\x54\x24\x70\x48\x89\x50\x60",
                )
                .expect("x64 patched signature"),
                // mov byte ptr [rax+8],1 ; nop ; nop
                fix: &[0xc6, 0x40, 0x08, 0x01, 0x90, 0x90],
            },
            BinaryGate {
                arch: Arch::Arm64,
                // ldrb w3,[x0,#8] ; tbz w3,#0,skip ; ldp x3,x4,[sp,#0x80] ; stp x3,x4,[x0,#0x60]
                original: Regex::new(
                    r"(?-u)\x03\x20\x40\x39[\x03\x23\x43\x63\x83\xa3\xc3\xe3].[\x00-\x07]\x36\xe3\x13\x48\xa9\x03\x10\x06\xa9",
                )
                .expect("arm64 original signature"),
                patched: Regex::new(
                    r"(?-u)\x23\x00\x80\x52\x03\x20\x00\x39\xe3\x13\x48\xa9\x03\x10\x06\xa9",
                )
                .expect("arm64 patched signature"),
                // mov w3,#1 ; strb w3,[x0,#8]
                fix: &[0x23, 0x00, 0x80, 0x52, 0x03, 0x20, 0x00, 0x39],
            },
        ]
    })
}

fn ide_original_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?-u)(resetIsTierGCPTos\(\),)this\.[A-Za-z_$0-9]+\.isGoogleInternal")
            .expect("ide original signature")
    })
}

fn ide_patched_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?-u)resetIsTierGCPTos\(\),true").expect("ide patched signature"))
}

fn is_macho(data: &[u8]) -> bool {
    data.len() >= 4
        && (data.starts_with(b"\xcf\xfa\xed\xfe")
            || data.starts_with(b"\xfe\xed\xfa\xcf")
            || data.starts_with(b"\xca\xfe\xba\xbe")
            || data.starts_with(b"\xbe\xba\xfe\xca")
            || data.starts_with(b"\xca\xfe\xba\xbf")
            || data.starts_with(b"\xbf\xba\xfe\xca"))
}

fn executable_info(
    data: &[u8],
    path: Option<&Path>,
    is_en: bool,
) -> Result<Vec<SliceInfo>, String> {
    if data.len() >= 2 && &data[..2] == b"MZ" {
        return pe_executable_info(data, is_en).map(|slice| vec![slice]);
    }
    if data.len() >= 4 && &data[..4] == b"\x7fELF" {
        return elf_executable_info(data, is_en).map(|slice| vec![slice]);
    }
    if is_macho(data) {
        return macho_executable_info(data, is_en);
    }
    let label = path
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "binary".into());
    Err(if is_en {
        format!("{label} is not a recognized PE/ELF/Mach-O executable.")
    } else {
        format!("{label} 不是可识别的 PE/ELF/Mach-O 可执行文件")
    })
}

fn checked_add(a: usize, b: usize, is_en: bool) -> Result<usize, String> {
    a.checked_add(b)
        .ok_or_else(|| msg(is_en, "二进制头偏移溢出", "Binary header offset overflow."))
}

fn checked_mul(a: usize, b: usize, is_en: bool) -> Result<usize, String> {
    a.checked_mul(b)
        .ok_or_else(|| msg(is_en, "二进制头大小溢出", "Binary header size overflow."))
}

fn pe_executable_info(data: &[u8], is_en: bool) -> Result<SliceInfo, String> {
    let pe = read_u32(data, 0x3c, is_en)? as usize;
    if read_bytes(data, pe, 4, is_en)? != b"PE\0\0" {
        return Err(msg(is_en, "无效的 PE 文件", "Invalid PE file."));
    }
    let coff = checked_add(pe, 4, is_en)?;
    let machine = read_u16(data, coff, is_en)?;
    let section_count = read_u16(data, checked_add(coff, 2, is_en)?, is_en)? as usize;
    let optional_size = read_u16(data, checked_add(coff, 16, is_en)?, is_en)? as usize;
    let section_table = checked_add(checked_add(pe, 24, is_en)?, optional_size, is_en)?;
    let mut ranges = Vec::new();
    for i in 0..section_count {
        let section = checked_add(section_table, checked_mul(i, 40, is_en)?, is_en)?;
        let raw_size = read_u32(data, checked_add(section, 16, is_en)?, is_en)? as usize;
        let raw_offset = read_u32(data, checked_add(section, 20, is_en)?, is_en)? as usize;
        let characteristics = read_u32(data, checked_add(section, 36, is_en)?, is_en)?;
        if characteristics & 0x2000_0000 != 0 && raw_size > 0 {
            ranges.push((raw_offset, raw_offset.saturating_add(raw_size)));
        }
    }
    Ok(SliceInfo {
        arch: machine_arch(machine as u64),
        ranges: normalize_ranges(ranges, data.len()),
    })
}

fn elf_executable_info(data: &[u8], is_en: bool) -> Result<SliceInfo, String> {
    if data.len() < 64 {
        return Err(msg(is_en, "ELF 文件过短", "ELF file is too short."));
    }
    let class = data[4];
    let little = match data[5] {
        1 => true,
        2 => false,
        _ => return Err(msg(is_en, "未知的 ELF 字节序", "Unknown ELF endianness.")),
    };
    let machine = read_int(data, 18, 2, little, is_en)?;
    let (phoff, phentsize, phnum, flags_off, file_off, size_off, wide) = if class == 2 {
        (
            read_int(data, 32, 8, little, is_en)? as usize,
            read_int(data, 54, 2, little, is_en)? as usize,
            read_int(data, 56, 2, little, is_en)? as usize,
            4usize,
            8usize,
            32usize,
            true,
        )
    } else if class == 1 {
        (
            read_int(data, 28, 4, little, is_en)? as usize,
            read_int(data, 42, 2, little, is_en)? as usize,
            read_int(data, 44, 2, little, is_en)? as usize,
            24usize,
            4usize,
            16usize,
            false,
        )
    } else {
        return Err(msg(is_en, "未知的 ELF class", "Unknown ELF class."));
    };
    let mut ranges = Vec::new();
    for i in 0..phnum {
        let entry = checked_add(phoff, checked_mul(i, phentsize, is_en)?, is_en)?;
        let p_type = read_int(data, entry, 4, little, is_en)?;
        let flags = read_int(data, checked_add(entry, flags_off, is_en)?, 4, little, is_en)?;
        let width = if wide { 8 } else { 4 };
        let offset = read_int(data, checked_add(entry, file_off, is_en)?, width, little, is_en)?
            as usize;
        let size = read_int(data, checked_add(entry, size_off, is_en)?, width, little, is_en)?
            as usize;
        if p_type == 1 && flags & 1 != 0 && size > 0 {
            ranges.push((offset, offset.saturating_add(size)));
        }
    }
    Ok(SliceInfo {
        arch: machine_arch(machine),
        ranges: normalize_ranges(ranges, data.len()),
    })
}

fn macho_executable_info(data: &[u8], is_en: bool) -> Result<Vec<SliceInfo>, String> {
    let magic = read_bytes(data, 0, 4, is_en)?;
    if magic == b"\xcf\xfa\xed\xfe" || magic == b"\xfe\xed\xfa\xcf" {
        return Ok(vec![macho_slice_info(data, 0, data.len(), is_en)?]);
    }
    let (little, is_64) = match magic {
        b"\xca\xfe\xba\xbe" => (false, false),
        b"\xbe\xba\xfe\xca" => (true, false),
        b"\xca\xfe\xba\xbf" => (false, true),
        b"\xbf\xba\xfe\xca" => (true, true),
        _ => return Err(msg(is_en, "不支持的 Mach-O 头", "Unsupported Mach-O header.")),
    };
    let count = read_int(data, 4, 4, little, is_en)? as usize;
    let entry_size = if is_64 { 32 } else { 20 };
    let mut slices = Vec::with_capacity(count.min(16));
    for i in 0..count {
        let entry = checked_add(8, checked_mul(i, entry_size, is_en)?, is_en)?;
        let (offset, size) = if is_64 {
            (
                read_int(data, checked_add(entry, 8, is_en)?, 8, little, is_en)? as usize,
                read_int(data, checked_add(entry, 16, is_en)?, 8, little, is_en)? as usize,
            )
        } else {
            (
                read_int(data, checked_add(entry, 8, is_en)?, 4, little, is_en)? as usize,
                read_int(data, checked_add(entry, 12, is_en)?, 4, little, is_en)? as usize,
            )
        };
        slices.push(macho_slice_info(data, offset, size, is_en)?);
    }
    if slices.is_empty() {
        return Err(msg(
            is_en,
            "Mach-O 中没有可用架构切片",
            "No architecture slices found in Mach-O binary.",
        ));
    }
    Ok(slices)
}

fn macho_slice_info(
    data: &[u8],
    base: usize,
    slice_size: usize,
    is_en: bool,
) -> Result<SliceInfo, String> {
    let magic = read_bytes(data, base, 4, is_en)?;
    let little = match magic {
        b"\xcf\xfa\xed\xfe" => true,
        b"\xfe\xed\xfa\xcf" => false,
        _ => {
            return Err(msg(
                is_en,
                "不支持的 Mach-O slice",
                "Unsupported Mach-O slice.",
            ))
        }
    };
    let arch = machine_arch(read_int(data, checked_add(base, 4, is_en)?, 4, little, is_en)?);
    let command_count = read_int(data, checked_add(base, 16, is_en)?, 4, little, is_en)? as usize;
    let mut pos = checked_add(base, 32, is_en)?;
    let slice_end = base.saturating_add(slice_size).min(data.len());
    let mut ranges = Vec::new();
    for _ in 0..command_count {
        let command = read_int(data, pos, 4, little, is_en)?;
        let command_size = read_int(data, checked_add(pos, 4, is_en)?, 4, little, is_en)? as usize;
        let next_pos = checked_add(pos, command_size, is_en)?;
        if command_size < 8 || next_pos > slice_end {
            return Err(msg(
                is_en,
                "无效的 Mach-O load command",
                "Invalid Mach-O load command.",
            ));
        }
        if command == 0x19 {
            let segment = read_bytes(data, pos, command_size, is_en)?;
            let section_count = read_int(segment, 64, 4, little, is_en)? as usize;
            let table_end = checked_add(72, checked_mul(section_count, 80, is_en)?, is_en)?;
            if table_end > command_size {
                return Err(msg(
                    is_en,
                    "无效的 Mach-O section table",
                    "Invalid Mach-O section table.",
                ));
            }
            for index in 0..section_count {
                let sec_start = 72 + index * 80;
                let section = &segment[sec_start..sec_start + 80];
                let section_name = &section[..16];
                let segment_name = &section[16..32];
                let size = read_int(section, 40, 8, little, is_en)? as usize;
                let offset = read_int(section, 48, 4, little, is_en)? as usize;
                let is_text =
                    segment_name.starts_with(b"__TEXT\0") && section_name.starts_with(b"__text\0");
                if is_text && size > 0 {
                    let range_start = checked_add(base, offset, is_en)?;
                    let range_end = range_start.saturating_add(size).min(slice_end);
                    ranges.push((range_start, range_end));
                }
            }
        }
        pos = next_pos;
    }
    if ranges.is_empty() {
        return Err(msg(
            is_en,
            "Mach-O 中没有可执行代码段",
            "No executable code section found in Mach-O binary.",
        ));
    }
    Ok(SliceInfo {
        arch,
        ranges: normalize_ranges(ranges, data.len()),
    })
}

fn machine_arch(machine: u64) -> Option<Arch> {
    match machine {
        0x8664 | 0x3e | 0x0100_0007 => Some(Arch::X64),
        0xaa64 | 0xb7 | 0x0100_000c => Some(Arch::Arm64),
        _ => None,
    }
}

fn normalize_ranges(mut ranges: Vec<(usize, usize)>, file_size: usize) -> Vec<(usize, usize)> {
    ranges.retain(|(start, end)| *start < *end && *start < file_size);
    for range in &mut ranges {
        range.1 = range.1.min(file_size);
    }
    ranges.sort_unstable();
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        if let Some(last) = out.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        out.push((start, end));
    }
    out
}

fn read_u16(data: &[u8], offset: usize, is_en: bool) -> Result<u16, String> {
    read_int(data, offset, 2, true, is_en).map(|v| v as u16)
}

fn read_u32(data: &[u8], offset: usize, is_en: bool) -> Result<u32, String> {
    read_int(data, offset, 4, true, is_en).map(|v| v as u32)
}

fn read_int(
    data: &[u8],
    offset: usize,
    size: usize,
    little: bool,
    is_en: bool,
) -> Result<u64, String> {
    let bytes = read_bytes(data, offset, size, is_en)?;
    let mut value = 0u64;
    if little {
        for (i, byte) in bytes.iter().enumerate() {
            value |= (*byte as u64) << (8 * i);
        }
    } else {
        for byte in bytes {
            value = (value << 8) | (*byte as u64);
        }
    }
    Ok(value)
}

fn read_bytes<'a>(data: &'a [u8], offset: usize, size: usize, is_en: bool) -> Result<&'a [u8], String> {
    let end = checked_add(offset, size, is_en)?;
    data.get(offset..end)
        .ok_or_else(|| msg(is_en, "读取超出文件范围", "Read out of file bounds."))
}

fn ensure_backup(target: &Path, original: &[u8], is_en: bool) -> Result<Vec<u8>, String> {
    let backup = backup_path(target);
    if backup.is_file() {
        let saved = fs::read(&backup).map_err(|e| {
            if is_en {
                format!("Failed to read existing backup: {e}")
            } else {
                format!("读取已有备份失败: {e}")
            }
        })?;
        if saved == original {
            return Ok(saved);
        }
        // An app update replaced the target with a new unpatched file.
        // Archive the previous backup aside (overwriting any older archived copy
        // so repeated updates work on Windows where rename fails if destination exists).
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        let stale = parent.join(format!(
            "{}.before-update{}",
            target.file_name().and_then(|n| n.to_str()).unwrap_or("target"),
            BACKUP_SUFFIX
        ));
        if stale.exists() {
            let _ = fs::remove_file(&stale);
        }
        fs::rename(&backup, &stale).map_err(|e| {
            if is_en {
                format!("Failed to archive previous backup: {e}")
            } else {
                format!("归档旧备份失败: {e}")
            }
        })?;
    }
    fs::write(&backup, original).map_err(|e| {
        if is_en {
            format!("Failed to create backup: {e}")
        } else {
            format!("创建备份失败: {e}")
        }
    })?;
    let saved = fs::read(&backup).map_err(|e| {
        if is_en {
            format!("Failed to verify backup: {e}")
        } else {
            format!("校验备份失败: {e}")
        }
    })?;
    if saved != original {
        return Err(msg(
            is_en,
            "备份校验失败，已中止修改。",
            "Backup verification failed; modification aborted.",
        ));
    }
    Ok(saved)
}

fn write_verified(path: &Path, bytes: &[u8], is_en: bool) -> Result<(), String> {
    let original_perms = fs::metadata(path).ok().map(|m| m.permissions());
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = parent.join(format!(
        ".{}.lar-tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("target")
    ));
    if tmp.exists() {
        tmp.set_extension(format!("lar-tmp-{}", std::process::id()));
    }
    {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)
            .map_err(|e| {
                if is_en {
                    format!("Failed to create temp file: {e}")
                } else {
                    format!("创建临时文件失败: {e}")
                }
            })?;
        file.write_all(bytes).map_err(|e| {
            if is_en {
                format!("Failed to write temp file: {e}")
            } else {
                format!("写入临时文件失败: {e}")
            }
        })?;
        file.flush().map_err(|e| {
            if is_en {
                format!("Failed to flush temp file: {e}")
            } else {
                format!("刷新临时文件失败: {e}")
            }
        })?;
    }
    if let Some(perms) = original_perms {
        let _ = fs::set_permissions(&tmp, perms);
    }
    let written = fs::read(&tmp).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        if is_en {
            format!("Failed to read back temp file: {e}")
        } else {
            format!("回读临时文件失败: {e}")
        }
    })?;
    if written != bytes {
        let _ = fs::remove_file(&tmp);
        return Err(msg(is_en, "临时文件校验失败", "Temp file verification failed."));
    }
    if let Err(rename_err) = fs::rename(&tmp, path) {
        let copy_res = fs::copy(&tmp, path);
        let _ = fs::remove_file(&tmp);
        if let Err(copy_err) = copy_res {
            return Err(if is_en {
                format!("Failed to replace target file: {rename_err}; {copy_err}")
            } else {
                format!("替换目标文件失败: {rename_err}; {copy_err}")
            });
        }
    }
    let current = fs::read(path).map_err(|e| {
        if is_en {
            format!("Failed to read back target file: {e}")
        } else {
            format!("回读目标文件失败: {e}")
        }
    })?;
    if current != bytes {
        return Err(msg(
            is_en,
            "目标文件写入后内容不一致",
            "Target file contents did not match after write.",
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn resign_macho_if_needed(path: &Path, data: &[u8], is_en: bool) -> Result<(), String> {
    if !is_macho(data) {
        return Ok(());
    }
    let output = std::process::Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(path)
        .output()
        .map_err(|e| {
            if is_en {
                format!("Failed to run codesign after patching (rolled back): {e}")
            } else {
                format!("补丁写入后执行 codesign 重签名失败，已回滚: {e}")
            }
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(if is_en {
            format!("codesign ad-hoc re-signing failed (rolled back): {stderr}")
        } else {
            format!("codesign 重签名失败，已回滚: {stderr}")
        });
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn resign_macho_if_needed(_path: &Path, _data: &[u8], _is_en: bool) -> Result<(), String> {
    Ok(())
}

fn restore_bytes(target: &Path, bytes: &[u8], is_en: bool) -> Result<(), String> {
    write_verified(target, bytes, is_en)
}

fn backup_path(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("target");
    target.with_file_name(format!("{name}{BACKUP_SUFFIX}"))
}

fn existing_backup(target: &Path) -> String {
    let path = backup_path(target);
    if path.is_file() {
        path.display().to_string()
    } else {
        String::new()
    }
}

fn is_write_locked(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    OpenOptions::new().read(true).write(true).open(path).is_err()
}

fn clear_ide_bytecode_cache() {
    for base in ide_user_data_dirs() {
        remove_dir_quiet(&base.join("CachedData"));
        remove_dir_quiet(&base.join("Code Cache").join("js"));
    }
}

fn ide_user_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let app_names = ["Antigravity IDE", "AntigravityIDE"];
    if let Some(roaming) = std::env::var_os("APPDATA") {
        let roaming = PathBuf::from(roaming);
        for name in app_names {
            dirs.push(roaming.join(name));
        }
    }
    if let Some(home) = dirs::home_dir() {
        let app_support = home.join("Library").join("Application Support");
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        for name in app_names {
            dirs.push(app_support.join(name));
            dirs.push(config.join(name));
        }
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            dirs.push(
                PathBuf::from(profile)
                    .join("scoop")
                    .join("persist")
                    .join("antigravity-ide")
                    .join("data")
                    .join("user-data"),
            );
        }
    }
    dirs
}

fn remove_dir_quiet(path: &Path) {
    if path.is_dir() {
        let _ = fs::remove_dir_all(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pe_with_section(code: &[u8]) -> Vec<u8> {
        let mut data = vec![0u8; 0x200];
        data[0] = b'M';
        data[1] = b'Z';
        data[0x3c] = 0x80;
        data[0x80..0x84].copy_from_slice(b"PE\0\0");
        data[0x84] = 0x64;
        data[0x85] = 0x86;
        data[0x86] = 1;
        data[0x94] = 0x10;
        let section = 0x80 + 24 + 0x10;
        data[section + 16..section + 20].copy_from_slice(&(code.len() as u32).to_le_bytes());
        data[section + 20..section + 24].copy_from_slice(&0x200u32.to_le_bytes());
        data[section + 36..section + 40].copy_from_slice(&0x6000_0020u32.to_le_bytes());
        data.extend_from_slice(code);
        data
    }

    fn x64_gate(patched: bool) -> Vec<u8> {
        let mut bytes = vec![
            0x80, 0x78, 0x08, 0x00, 0x74, 0x12, 0x48, 0x8b, 0x54, 0x24, 0x70, 0x48, 0x89, 0x50,
            0x60,
        ];
        if patched {
            bytes[..6].copy_from_slice(&[0xc6, 0x40, 0x08, 0x01, 0x90, 0x90]);
        }
        bytes
    }

    fn arm64_gate(patched: bool) -> Vec<u8> {
        let mut bytes = vec![
            0x03, 0x20, 0x40, 0x39, 0x43, 0x02, 0x03, 0x36, 0xe3, 0x13, 0x48, 0xa9, 0x03, 0x10,
            0x06, 0xa9,
        ];
        if patched {
            bytes[..8].copy_from_slice(&[0x23, 0x00, 0x80, 0x52, 0x03, 0x20, 0x00, 0x39]);
        }
        bytes
    }

    fn thin_macho(cputype: u32, code: &[u8]) -> Vec<u8> {
        // 32-byte mach_header_64 + 152-byte LC_SEGMENT_64 (72 + 80) = 184 bytes
        let code_off = 0x100usize;
        let mut buf = vec![0u8; code_off];
        buf[0..4].copy_from_slice(b"\xcf\xfa\xed\xfe");
        buf[4..8].copy_from_slice(&cputype.to_le_bytes());
        buf[16..20].copy_from_slice(&1u32.to_le_bytes()); // ncmds = 1
        let cmd_pos = 32;
        buf[cmd_pos..cmd_pos + 4].copy_from_slice(&0x19u32.to_le_bytes()); // LC_SEGMENT_64
        buf[cmd_pos + 4..cmd_pos + 8].copy_from_slice(&152u32.to_le_bytes());
        buf[cmd_pos + 64..cmd_pos + 68].copy_from_slice(&1u32.to_le_bytes()); // nsects = 1
        let sec = cmd_pos + 72;
        buf[sec..sec + 7].copy_from_slice(b"__text\0");
        buf[sec + 16..sec + 23].copy_from_slice(b"__TEXT\0");
        buf[sec + 40..sec + 48].copy_from_slice(&(code.len() as u64).to_le_bytes());
        buf[sec + 48..sec + 52].copy_from_slice(&(code_off as u32).to_le_bytes());
        buf.extend_from_slice(code);
        buf
    }

    fn fat_macho(slices: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let header_size = 8 + slices.len() * 20;
        let mut out = vec![0u8; header_size];
        out[0..4].copy_from_slice(b"\xca\xfe\xba\xbe");
        out[4..8].copy_from_slice(&(slices.len() as u32).to_be_bytes());
        for (i, (cputype, slice_bytes)) in slices.iter().enumerate() {
            let offset = out.len() as u32;
            let size = slice_bytes.len() as u32;
            let entry = 8 + i * 20;
            out[entry..entry + 4].copy_from_slice(&cputype.to_be_bytes());
            out[entry + 8..entry + 12].copy_from_slice(&offset.to_be_bytes());
            out[entry + 12..entry + 16].copy_from_slice(&size.to_be_bytes());
            out.extend_from_slice(slice_bytes);
        }
        out
    }

    #[test]
    fn detects_and_patches_only_the_x64_gate() {
        let file = pe_with_section(&x64_gate(false));
        let slices = executable_info(&file, None, false).unwrap();
        let (status, sites) = inspect_binary(&file, &slices, false).unwrap();
        assert_eq!(status, PatchStatus::Unpatched);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].offset, 0x200);

        let mut patched = file.clone();
        let fix = manager_gates()[sites[0].gate_index].fix;
        patched[sites[0].offset..sites[0].offset + fix.len()].copy_from_slice(fix);
        let (status, _) = inspect_binary(&patched, &slices, false).unwrap();
        assert_eq!(status, PatchStatus::Patched);
    }

    #[test]
    fn supports_universal_macho_fat_binary() {
        let x64_slice = thin_macho(0x0100_0007, &x64_gate(false));
        let arm_slice = thin_macho(0x0100_000c, &arm64_gate(false));
        let fat = fat_macho(&[(0x0100_0007, x64_slice), (0x0100_000c, arm_slice)]);

        let slices = executable_info(&fat, None, false).unwrap();
        assert_eq!(slices.len(), 2);
        let (status, sites) = inspect_binary(&fat, &slices, false).unwrap();
        assert_eq!(status, PatchStatus::Unpatched);
        assert_eq!(sites.len(), 2);

        let mut patched = fat.clone();
        for site in &sites {
            let fix = manager_gates()[site.gate_index].fix;
            patched[site.offset..site.offset + fix.len()].copy_from_slice(fix);
        }
        let (status, verified) = inspect_binary(&patched, &slices, false).unwrap();
        assert_eq!(status, PatchStatus::Patched);
        assert_eq!(verified.len(), 2);
    }

    #[test]
    fn refuses_duplicate_binary_signatures() {
        let mut code = x64_gate(false);
        code.extend_from_slice(&x64_gate(false));
        let file = pe_with_section(&code);
        let slices = executable_info(&file, None, false).unwrap();
        let err = inspect_binary(&file, &slices, false).unwrap_err();
        assert!(err.contains("不唯一"));
    }

    #[test]
    fn handles_malformed_overflow_headers_without_panic() {
        let mut bad_pe = vec![0u8; 0x100];
        bad_pe[0..2].copy_from_slice(b"MZ");
        bad_pe[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(executable_info(&bad_pe, None, true).is_err());
    }

    #[test]
    fn ensure_backup_handles_repeated_updates() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("language_server.exe");
        let v1 = pe_with_section(&x64_gate(false));
        let mut v2 = pe_with_section(&x64_gate(false));
        v2.push(0x90);
        let mut v3 = pe_with_section(&x64_gate(false));
        v3.extend_from_slice(&[0x90, 0x90]);

        fs::write(&target, &v1).unwrap();
        ensure_backup(&target, &v1, false).unwrap();
        fs::write(&target, &v2).unwrap();
        ensure_backup(&target, &v2, false).unwrap();
        fs::write(&target, &v3).unwrap();
        let saved = ensure_backup(&target, &v3, false).unwrap();
        assert_eq!(saved, v3);
    }

    #[test]
    fn patches_ide_gate_without_touching_other_text() {
        let source = b"prefix resetIsTierGCPTos(),this.abc.isGoogleInternal suffix";
        let patched = replace_ide_gate(source, false).unwrap();
        assert_eq!(patched, b"prefix resetIsTierGCPTos(),true suffix");
        assert_eq!(inspect_ide(&patched, false).unwrap().0, PatchStatus::Patched);
    }

    #[test]
    fn refuses_missing_or_duplicate_ide_gate() {
        assert!(inspect_ide(b"no gate here", false).is_err());
        let duplicated = b"resetIsTierGCPTos(),this.a.isGoogleInternal resetIsTierGCPTos(),this.b.isGoogleInternal";
        assert!(replace_ide_gate(duplicated, false).is_err());
    }
}
