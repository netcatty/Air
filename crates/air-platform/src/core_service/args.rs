use std::path::PathBuf;

use air_error::{AppResult, PlatformError};

use super::types::{CORE_SERVICE_ARG, CoreServicePaths, SERVICE_OWNER_PID_ARG};
pub fn service_binary_args(paths: &CoreServicePaths) -> AppResult<Vec<String>> {
    let exe = std::env::current_exe().map_err(|error| {
        PlatformError::OperationFailed(format!("读取当前程序路径失败: {error}"))
    })?;
    Ok(vec![
        exe.to_string_lossy().to_string(),
        CORE_SERVICE_ARG.to_string(),
        "--config-dir".to_string(),
        paths.config_dir.to_string_lossy().to_string(),
        "--data-dir".to_string(),
        paths.data_dir.to_string_lossy().to_string(),
        "--cache-dir".to_string(),
        paths.cache_dir.to_string_lossy().to_string(),
    ])
}

pub(super) fn service_start_args(owner_pid: u32) -> Vec<String> {
    vec![SERVICE_OWNER_PID_ARG.to_string(), owner_pid.to_string()]
}
#[cfg(windows)]
pub(super) unsafe fn service_args_from_argv(argc: u32, argv: *mut *mut u16) -> Vec<String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    if argv.is_null() {
        return Vec::new();
    }
    (0..argc)
        .filter_map(|index| {
            let ptr = unsafe { *argv.add(index as usize) };
            if ptr.is_null() {
                return None;
            }
            let mut len = 0usize;
            while unsafe { *ptr.add(len) } != 0 {
                len += 1;
            }
            let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
            Some(OsString::from_wide(slice).to_string_lossy().to_string())
        })
        .collect()
}

pub(super) fn service_owner_pid_from_args(args: &[String]) -> Option<u32> {
    args.windows(2).find_map(|pair| {
        (pair[0] == SERVICE_OWNER_PID_ARG)
            .then(|| pair[1].parse::<u32>().ok())
            .flatten()
            .filter(|pid| *pid > 0)
    })
}

/// 解析服务运行时路径：优先使用 `resolve_default()` 动态解析（支持便携模式移动后自愈），
/// 仅在动态解析失败时回退到命令行参数（兼容旧版注册和非便携模式）。
///
/// 便携模式下用户可能将程序文件夹移动到新位置，此时 SCM 注册的 ImagePath 仍指向旧路径。
/// 服务 worker 调用此函数可以在启动时自动使用正确的新路径，无需手动卸载重装。
pub(super) fn resolve_service_paths() -> AppResult<CoreServicePaths> {
    // 策略 1：运行时动态解析。便携模式下 portable_root() 基于当前 exe 位置推导，
    // 即使 SCM 注册路径过期也能返回正确目录。
    if let Ok(resolved) = CoreServicePaths::resolve_default() {
        // 先确保目录存在（移动后首次启动可能目录尚未创建），再验证关键目录可用。
        if let Ok(()) = resolved.init() {
            if resolved.key_dirs_exist() {
                tracing::info!(
                    config_dir = %resolved.config_dir.display(),
                    cores_dir = %resolved.cores_dir.display(),
                    "服务路径已通过运行时解析确定"
                );
                return Ok(resolved);
            }
        }
    }

    // 策略 2：回退到命令行参数。覆盖非便携模式（系统目录）和 resolve_default()
    // 意外失败的极端情况。
    let paths = parse_paths_from_args()?;
    tracing::info!(
        config_dir = %paths.config_dir.display(),
        cores_dir = %paths.cores_dir.display(),
        "服务路径已通过命令行参数解析"
    );
    Ok(paths)
}

/// 从命令行参数解析服务路径（旧版行为，作为动态解析的回退）。
fn parse_paths_from_args() -> AppResult<CoreServicePaths> {
    let mut config_dir = None;
    let mut data_dir = None;
    let mut cache_dir = None;
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--config-dir" => config_dir = iter.next().map(PathBuf::from),
            "--data-dir" => data_dir = iter.next().map(PathBuf::from),
            "--cache-dir" => cache_dir = iter.next().map(PathBuf::from),
            _ => {}
        }
    }
    Ok(CoreServicePaths::from_base_dirs(
        &config_dir.ok_or_else(|| PlatformError::OperationFailed("服务缺少配置目录".into()))?,
        &data_dir.ok_or_else(|| PlatformError::OperationFailed("服务缺少数据目录".into()))?,
        &cache_dir.ok_or_else(|| PlatformError::OperationFailed("服务缺少缓存目录".into()))?,
    ))
}
#[cfg(windows)]
pub(super) fn wide_os_null(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
pub(super) fn wide_null(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
