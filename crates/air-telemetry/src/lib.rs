extern crate self as air_telemetry;

pub mod log_retention;
pub mod memory;
pub mod redaction;

use std::sync::Once;

use tracing_subscriber::EnvFilter;

static INIT: Once = Once::new();

pub fn init_tracing() {
    INIT.call_once(|| {
        let filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("air=info"));
        #[cfg(debug_assertions)]
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            // 应用日志面向控制台、文件和外部诊断工具，固定关闭 ANSI 颜色，避免输出转义码影响检索和复制。
            .with_ansi(false)
            .try_init();
        #[cfg(not(debug_assertions))]
        {
            match file_log_writer() {
                Ok(writer) => {
                    let _ = tracing_subscriber::fmt()
                        .with_env_filter(filter)
                        .with_target(false)
                        // release 默认写入 air.log；文件日志必须保持纯文本，不能依赖终端颜色探测。
                        .with_ansi(false)
                        .with_writer(writer)
                        .try_init();
                }
                Err(error) => {
                    eprintln!("failed to initialize file logging: {error}");
                    let _ = tracing_subscriber::fmt()
                        .with_env_filter(filter)
                        .with_target(false)
                        // 文件日志初始化失败时回退到 stderr，同样保持纯文本输出。
                        .with_ansi(false)
                        .try_init();
                }
            }
        }
    });
}

#[cfg(not(debug_assertions))]
fn file_log_writer() -> std::io::Result<FileLogWriter> {
    use std::fs::OpenOptions;
    use std::sync::{Arc, Mutex};

    // 便携模式日志路径：与 air-storage::paths 和 air-platform::core_service 保持一致，
    // 优先使用 exe 同级 data/ 子目录下的 logs/，避免 release 构建仍写入系统 AppData。
    // air-telemetry 不依赖 air-storage，这里独立复制检测逻辑。
    let logs_dir = portable_logs_dir()?;
    std::fs::create_dir_all(&logs_dir)?;
    let path = logs_dir.join("air.log");
    tracing::info!(log_path = %path.display(), "release file log writer initialized");
    log_retention::prepare_managed_log_for_append(&path)?;
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    Ok(FileLogWriter {
        inner: Arc::new(Mutex::new(FileLogInner {
            file,
            path,
            active_date: log_retention::local_now().date(),
        })),
    })
}

/// 便携模式数据根目录名，与 air-storage::paths::PORTABLE_DIR_NAME 保持一致。
#[cfg(not(debug_assertions))]
const PORTABLE_DIR_NAME: &str = "data";

/// 返回日志目录：优先便携模式（exe 同级 data/data/logs），回退到系统标准目录。
/// air-telemetry 不依赖 air-storage，独立复制便携检测逻辑以保持行为一致。
#[cfg(not(debug_assertions))]
fn portable_logs_dir() -> std::io::Result<std::path::PathBuf> {
    // 便携模式：exe 同级 data/ 子目录作为数据根，日志放在 data/data/logs。
    if let Some(root) = portable_root() {
        return Ok(root.join("data").join("logs"));
    }
    // 理论上极少触发；current_exe 不可用时回退到操作系统标准用户目录。
    let project_dirs = directories::ProjectDirs::from("org.air", "", "Air").ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::Other, "project directories unavailable")
    })?;
    Ok(project_dirs.data_dir().join("logs"))
}

/// 返回便携模式数据根目录：exe 同级 data/ 子目录。
#[cfg(not(debug_assertions))]
fn portable_root() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    Some(exe_dir.join(PORTABLE_DIR_NAME))
}

#[cfg(not(debug_assertions))]
#[derive(Clone)]
struct FileLogWriter {
    inner: std::sync::Arc<std::sync::Mutex<FileLogInner>>,
}

#[cfg(not(debug_assertions))]
struct FileLogGuard {
    inner: std::sync::Arc<std::sync::Mutex<FileLogInner>>,
}

#[cfg(not(debug_assertions))]
struct FileLogInner {
    file: std::fs::File,
    path: std::path::PathBuf,
    active_date: time::Date,
}

#[cfg(not(debug_assertions))]
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for FileLogWriter {
    type Writer = FileLogGuard;

    fn make_writer(&'a self) -> Self::Writer {
        FileLogGuard {
            inner: std::sync::Arc::clone(&self.inner),
        }
    }
}

#[cfg(not(debug_assertions))]
impl std::io::Write for FileLogGuard {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut inner = self.inner.lock().map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::Other, "log file lock poisoned")
        })?;
        inner.rotate_if_needed()?;
        inner.file.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner
            .lock()
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "log file lock poisoned"))?
            .file
            .flush()
    }
}

#[cfg(not(debug_assertions))]
impl FileLogInner {
    fn rotate_if_needed(&mut self) -> std::io::Result<()> {
        let now = log_retention::local_now();
        if now.date() == self.active_date {
            return Ok(());
        }
        std::io::Write::flush(&mut self.file)?;
        log_retention::prepare_managed_log_for_append_at(&self.path, now)?;
        self.file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.active_date = now.date();
        Ok(())
    }
}
