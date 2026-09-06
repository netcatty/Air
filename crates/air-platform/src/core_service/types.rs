use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use air_error::AppResult;
pub const CORE_SERVICE_NAME: &str = "AirMihomoCore";
pub const CORE_SERVICE_DISPLAY_NAME: &str = "Air Mihomo Core Service";
pub(super) const CORE_SERVICE_ARG: &str = "--air-mihomo-service";
pub(super) const ELEVATED_SERVICE_HELPER_ARG: &str = "--air-elevated-service-helper";
pub(super) const SERVICE_OWNER_PID_ARG: &str = "--owner-pid";
pub(super) const SERVICE_ADMIN_RIGHTS_SDDL: &str = "CCDCLCSWRPWPDTLOCRSDRCWDWO";
pub(super) const SERVICE_INTERACTIVE_USER_RIGHTS_SDDL: &str = "LCRPWP";

// Windows 标准访问位不属于服务模块本身；这里显式保留数值，避免为了少量 ACL 掩码引入额外
// windows-sys feature。它们分别对应 DELETE / READ_CONTROL / WRITE_DAC / WRITE_OWNER。
#[cfg(windows)]
pub(super) const STANDARD_DELETE: u32 = 0x0001_0000;
#[cfg(windows)]
pub(super) const STANDARD_READ_CONTROL: u32 = 0x0002_0000;
#[cfg(windows)]
pub(super) const STANDARD_WRITE_DAC: u32 = 0x0004_0000;
#[cfg(windows)]
pub(super) const STANDARD_WRITE_OWNER: u32 = 0x0008_0000;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CoreServiceSnapshot {
    pub installed: bool,
    pub running: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreServiceAction {
    Install,
    Uninstall,
    /// 修复服务注册路径：仅更新 ImagePath，不卸载重装，不需要停止服务。
    Repair,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreServicePaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub cores_dir: PathBuf,
    pub logs_dir: PathBuf,
}

impl CoreServicePaths {
    pub fn from_base_dirs(config_dir: &Path, data_dir: &Path, cache_dir: &Path) -> Self {
        Self {
            config_dir: config_dir.to_path_buf(),
            data_dir: data_dir.to_path_buf(),
            cache_dir: cache_dir.to_path_buf(),
            cores_dir: cache_dir.join("core"),
            logs_dir: data_dir.join("logs"),
        }
    }

    pub fn resolve_default() -> AppResult<Self> {
        // 便携模式：exe 同级 data/ 子目录即数据根，与 AppPaths 保持一致，
        // 避免 GUI 和服务化核心进程使用不同的数据目录。
        // air-platform 不依赖 air-storage，这里独立复制检测逻辑。
        if let Some(root) = portable_root() {
            return Ok(Self::from_base_dirs(
                &root.join("config"),
                &root.join("data"),
                &root.join("cache"),
            ));
        }
        let project_dirs = directories::ProjectDirs::from("org.air", "", "Air")
            .ok_or(air_error::StorageError::ProjectDirsUnavailable)?;
        Ok(Self::from_base_dirs(
            project_dirs.config_dir(),
            project_dirs.data_dir(),
            project_dirs.cache_dir(),
        ))
    }

    /// 检查关键目录是否已在磁盘上存在。
    /// 便携模式下移动文件夹后，旧路径的目录会消失，新路径尚不存在；
    /// 此方法用于服务 worker 判断 `resolve_default()` 解析出的路径是否可用。
    pub fn key_dirs_exist(&self) -> bool {
        // config_dir 是 mihomo 运行配置的存放位置；cores_dir 是核心二进制所在位置。
        // 二者缺一不可；logs_dir 和 backups_dir 可以在运行时自动创建。
        self.config_dir.is_dir() && self.cores_dir.is_dir()
    }

    pub(super) fn init(&self) -> AppResult<()> {
        for dir in [
            &self.config_dir,
            &self.data_dir,
            &self.cache_dir,
            &self.cores_dir,
            &self.logs_dir,
        ] {
            std::fs::create_dir_all(dir).map_err(air_error::StorageError::Io)?;
        }
        Ok(())
    }
}

impl CoreServiceAction {
    pub(super) fn as_arg(self) -> &'static str {
        match self {
            Self::Install => "--install",
            Self::Uninstall => "--uninstall",
            Self::Repair => "--repair",
        }
    }

    pub(super) fn from_arg(value: &str) -> Option<Self> {
        match value {
            "--install" => Some(Self::Install),
            "--uninstall" => Some(Self::Uninstall),
            "--repair" => Some(Self::Repair),
            _ => None,
        }
    }
}

/// 返回便携模式数据根目录：exe 同级 data/ 子目录。
/// air-platform 不依赖 air-storage，这里独立复制检测逻辑，保持两边行为一致。
fn portable_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    Some(exe_dir.join("data"))
}
