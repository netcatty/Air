use std::path::{Path, PathBuf};

use air_error::{AppResult, StorageError};

// 保留 directories 用于 current_exe 不可用时的系统目录回退。
#[allow(unused_imports)]
use directories::ProjectDirs;

/// 便携模式数据目录名：exe 同级该子目录为数据根，所有文件集中存放于此。
const PORTABLE_DIR_NAME: &str = "data";

/// 返回便携模式数据根目录：exe 同级 data/ 子目录。
/// 默认便携策略：不要求 data/ 目录已存在，子目录在 init() 时自动创建。
/// 用户只需双击 exe 即可，无需任何参数或预建目录。
fn portable_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    Some(exe_dir.join(PORTABLE_DIR_NAME))
}

/// 解析数据根目录。默认便携模式：exe 同级 data/ 子目录；
/// current_exe 不可用时回退到操作系统标准用户目录。
fn resolve_base() -> AppResult<PathBuf> {
    if let Some(root) = portable_root() {
        return Ok(root);
    }
    // 理论上极少触发；保留系统目录回退避免完全无路径可用。
    let dirs =
        ProjectDirs::from("org.air", "", "Air").ok_or(StorageError::ProjectDirsUnavailable)?;
    Ok(dirs.data_dir().to_path_buf())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub subscription_cache_dir: PathBuf,
    pub cores_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub backups_dir: PathBuf,
}

impl AppPaths {
    /// 解析应用路径：默认便携模式（exe 同级 data/），回退到系统目录。
    pub fn resolve() -> AppResult<Self> {
        let root = resolve_base()?;
        let paths = Self::from_base_dirs(
            &root.join("config"),
            &root.join("data"),
            &root.join("cache"),
        );
        tracing::info!(
            config_dir = %paths.config_dir.display(),
            data_dir = %paths.data_dir.display(),
            cache_dir = %paths.cache_dir.display(),
            "resolved application paths"
        );
        Ok(paths)
    }

    /// 返回便携模式数据根目录，供 CoreServicePaths 复用。
    pub fn portable_root() -> Option<PathBuf> {
        portable_root()
    }

    pub fn from_base_dirs(config_dir: &Path, data_dir: &Path, cache_dir: &Path) -> Self {
        // Windows/macOS/Linux 的系统目录不同，但业务层只依赖这些语义化子目录。
        Self {
            config_dir: config_dir.to_path_buf(),
            data_dir: data_dir.to_path_buf(),
            cache_dir: cache_dir.to_path_buf(),
            subscription_cache_dir: config_dir.join("subscriptions"),
            cores_dir: cache_dir.join("core"),
            logs_dir: data_dir.join("logs"),
            backups_dir: data_dir.join("backups"),
        }
    }

    pub fn init(&self) -> AppResult<()> {
        for dir in [
            &self.config_dir,
            &self.data_dir,
            &self.cache_dir,
            &self.subscription_cache_dir,
            &self.cores_dir,
            &self.logs_dir,
            &self.backups_dir,
        ] {
            std::fs::create_dir_all(dir).map_err(StorageError::Io)?;
            tracing::debug!(path = %dir.display(), "ensured application directory exists");
        }
        tracing::info!("initialized application directory layout");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_semantic_subdirectories_from_platform_roots() {
        let paths = AppPaths::from_base_dirs(
            Path::new("/config/air"),
            Path::new("/data/air"),
            Path::new("/cache/air"),
        );

        assert_eq!(
            paths.subscription_cache_dir,
            PathBuf::from("/config/air/subscriptions")
        );
        assert_eq!(paths.cores_dir, PathBuf::from("/cache/air/core"));
        assert_eq!(paths.backups_dir, PathBuf::from("/data/air/backups"));
    }

    #[test]
    fn keeps_same_layout_for_common_platform_roots() {
        // 目录库会按平台返回不同根目录；这里验证业务子目录在三类根目录下保持一致。
        for (config, data, cache) in [
            (
                r"C:\Users\Alice\AppData\Roaming\dev\air\air\config",
                r"C:\Users\Alice\AppData\Roaming\dev\air\air\data",
                r"C:\Users\Alice\AppData\Local\dev\air\air\cache",
            ),
            (
                "/Users/alice/Library/Application Support/dev.air.air",
                "/Users/alice/Library/Application Support/dev.air.air",
                "/Users/alice/Library/Caches/dev.air.air",
            ),
            (
                "/home/alice/.config/air",
                "/home/alice/.local/share/air",
                "/home/alice/.cache/air",
            ),
        ] {
            let paths =
                AppPaths::from_base_dirs(Path::new(config), Path::new(data), Path::new(cache));

            assert!(paths.subscription_cache_dir.ends_with("subscriptions"));
            assert!(paths.cores_dir.ends_with("core"));
            assert!(paths.logs_dir.ends_with("logs"));
            assert!(paths.backups_dir.ends_with("backups"));
        }
    }
}
