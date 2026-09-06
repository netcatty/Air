use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rerun-if-changed={}",
        manifest_path("assets/app-icon.png").display()
    );

    if target_is_windows() {
        embed_windows_application_resources();
    }
}

fn target_is_windows() -> bool {
    env::var("TARGET")
        .map(|target| target.contains("windows"))
        .unwrap_or(false)
}

/// Windows 可执行文件资源在构建期统一嵌入：图标 + 版本信息。
///
/// 版本信息里的 `FileDescription` 固定为 "Air"。任务管理器在进程没有可见窗口
/// （例如静默启动隐藏到托盘、最小化到托盘）时会用 `FileDescription` 作为进程显示名，
/// 这样就不会回退到 `air.exe`，与窗口可见时显示的窗口标题 "Air" 保持一致。
fn embed_windows_application_resources() {
    // Windows 可执行文件图标在构建期从 PNG 转成 ICO 并嵌入资源，保持仓库内只维护单一源素材。
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR should be set by cargo"));
    let icon_png = manifest_path("assets/app-icon.png");
    let icon_ico = out_dir.join("air-app-icon.ico");
    let resource_rc = out_dir.join("air-app-icon.rc");

    let icon = image::open(&icon_png)
        .unwrap_or_else(|error| panic!("failed to load {}: {error}", icon_png.display()));
    // ICO 单张位图最大 256px，这里在构建期统一缩放，避免额外维护独立 ico 文件。
    icon.thumbnail(256, 256)
        .save_with_format(&icon_ico, image::ImageFormat::Ico)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", icon_ico.display()));

    let icon_ico = icon_ico.to_string_lossy().replace('\\', "/");
    let version_info = windows_version_info_rc();
    // 图标与版本信息写入同一份 RC，一次编译嵌入，避免多份资源脚本相互覆盖。
    let rc_contents = format!("1 ICON \"{icon_ico}\"\n{version_info}");
    fs::write(&resource_rc, rc_contents)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", resource_rc.display()));

    embed_resource::compile(&resource_rc, embed_resource::NONE)
        .manifest_optional()
        .unwrap_or_else(|error| panic!("failed to compile {}: {error}", resource_rc.display()));
}

/// 生成 `VERSIONINFO` 资源块。
///
/// 任务管理器在进程没有可见窗口时，会用版本信息的 `FileDescription` 作为进程显示名；
/// 这里固定为 "Air"，与窗口标题一致，避免启动到托盘等场景下显示 `air.exe`。
fn windows_version_info_rc() -> String {
    let [major, minor, patch, build] = version_components();
    let file_version = format!("{major}.{minor}.{patch}.{build}");

    // 0x0409 = English (US)，0x04B0 = Unicode(1200)；这是 Windows 资源约定俗成的代码页块。
    // FILEOS 0x40004L = VOS_NT_WINDOWS32，FILETYPE 0x1L = VFT_APP，用数值常量避免依赖 windows 头。
    format!(
        r#"1 VERSIONINFO
FILEVERSION {major},{minor},{patch},{build}
PRODUCTVERSION {major},{minor},{patch},{build}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "Air"
      VALUE "FileDescription", "Air"
      VALUE "FileVersion", "{file_version}"
      VALUE "InternalName", "air"
      VALUE "OriginalFilename", "air.exe"
      VALUE "ProductName", "Air"
      VALUE "ProductVersion", "{file_version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    )
}

/// 将 `CARGO_PKG_VERSION` 拆成 4 段，不足部分补 0，用于 `FILEVERSION` / `PRODUCTVERSION`。
fn version_components() -> [u16; 4] {
    let mut components = [0u16; 4];
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    for (index, part) in version.split('.').take(4).enumerate() {
        if let Ok(value) = part.parse::<u16>() {
            components[index] = value;
        }
    }
    components
}

fn manifest_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}
