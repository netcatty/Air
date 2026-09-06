use std::time::Duration;

use air_app::RuntimeStatus;
use air_app::runtime::CancellationToken;
use air_app::services::AppServices;
use air_error::{AppResult, RuntimeError};

pub(super) fn ensure_not_canceled(token: &CancellationToken) -> AppResult<()> {
    if token.is_canceled() {
        Err(RuntimeError::Canceled.into())
    } else {
        Ok(())
    }
}

pub(super) async fn wait_for_cancellation(token: CancellationToken) {
    while !token.is_canceled() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub(super) fn runtime_is_running(services: &AppServices) -> bool {
    matches!(
        services.snapshots.snapshot().runtime,
        RuntimeStatus::Running
    )
}

pub(super) fn runtime_api_available(services: &AppServices) -> bool {
    runtime_is_running(services)
}

pub(super) async fn reload_runtime_config_if_running(services: &AppServices) -> AppResult<()> {
    if !runtime_is_running(services) {
        return Ok(());
    }
    // UI 保存的是用户配置；mihomo 运行时实际读取合并后的 runtime 配置。
    // 这里先重新生成 runtime 配置，再让核心通过 PUT /configs 重载当前配置路径。
    services.write_runtime_config_validated().await?;
    reload_current_runtime_config_if_running(services).await
}

pub(super) async fn reload_current_runtime_config_if_running(
    services: &AppServices,
) -> AppResult<()> {
    if !runtime_is_running(services) {
        return Ok(());
    }
    // 使用 payload 字段直接传入配置内容，而非 path 字段传入配置文件路径。
    // 便携模式下移动文件夹后，mihomo 进程的 SAFE_PATHS 仍指向旧路径，
    // 新路径的配置文件无法通过 mihomo 的 IsSafePath 安全检查；
    // payload 方式绕过路径限制，同时避免把配置正文暴露进 API 日志（由 mihomo 端校验）。
    let runtime_config_path = services.core_config_store.runtime_config_path();
    let runtime_config_content = std::fs::read_to_string(&runtime_config_path)
        .map_err(|error| air_error::StorageError::Io(error))?;
    services
        .mihomo_clients
        .client()?
        .reload_configs(false, "", &runtime_config_content)
        .await
}

pub(super) async fn apply_current_mihomo_status(services: &AppServices) {
    // 启动失败可能发生在 spawn、配置解析或健康检查任一阶段；失败路径也要回填快照，
    // 否则 UI 会停留在点击前状态，后续按钮状态和错误展示都会失真。
    if let Ok(status) = services.mihomo.status().await {
        services.apply_mihomo_status(status);
    }
}

pub(super) async fn current_core_version(services: &AppServices) -> AppResult<Option<String>> {
    if let Some(version) = services
        .snapshots
        .snapshot()
        .runtime_info
        .and_then(|runtime| runtime.version)
    {
        return Ok(Some(version));
    }

    // 订阅下载 UA 要和当前核心版本保持一致；若启动后尚未检测过核心，这里只刷新检测投影，不启动进程。
    services.detect_core().await?;
    Ok(services
        .snapshots
        .snapshot()
        .runtime_info
        .and_then(|runtime| runtime.version))
}
