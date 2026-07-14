//! 认证命令。

use serde::Serialize;
use tauri::AppHandle;

use crate::auth::models::{TokenPair, UserInfo};
use crate::auth::token_store::TokenStore;
use crate::auth::user_info_api::UserInfoApi;
use crate::core::config_store::ConfigStore;
use crate::data::repository;
use crate::error::AppResult;

use super::{drop_runtime_async, AUTH_SERVICE, DB};

/// 仅清理数据库同步行与缓存文件，不删除当前数据库文件或新 token。
fn clear_account_caches() {
    {
        let conn = DB.lock();
        let _ = repository::delete_all(&conn);
        let _ = repository::delete_all_transfers(&conn);
    }
    crate::core::cache_paths::clear_all_cache_files();
}

/// 前端恢复认证页面所需的登录、凭据与回调端口快照。
#[derive(Debug, Clone, Serialize)]
pub struct AuthState {
    pub logged_in: bool,
    pub secret_configured: bool,
    pub callback_port: u16,
}

/// 检查 OAuth 客户端标识与密钥是否同时完成配置。
#[tauri::command]
pub fn auth_check_secret() -> bool {
    crate::constants::client_id_configured() && crate::constants::client_secret_configured()
}

/// 从 token store 恢复登录状态，并返回当前认证配置快照。
#[tauri::command]
pub async fn auth_restore() -> AppResult<AuthState> {
    let logged_in = AUTH_SERVICE.restore().await?;
    Ok(AuthState {
        logged_in,
        secret_configured: crate::constants::client_id_configured()
            && crate::constants::client_secret_configured(),
        callback_port: crate::constants::DEFAULT_CALLBACK_PORT,
    })
}

/// 完成 OAuth 登录，并在切换账号后停止旧运行时、清理同步数据和重置目录配置。
#[tauri::command]
pub async fn auth_login(app: AppHandle, port: u16) -> AppResult<TokenPair> {
    let token = AUTH_SERVICE.authorize(port).await?;
    // 清空旧账号同步状态
    drop_runtime_async().await;
    clear_account_caches();
    let _ = reset_account_config();
    tracing::info!("登录成功，已彻底清空上一账号同步缓存与目录配置，等待用户重新配置");
    // 保留 AppHandle 以兼容命令签名
    let _ = &app;
    Ok(token)
}

/// 清空挂载目录配置，避免新账号复用上一账号的同步目录。
fn reset_account_config() -> AppResult<()> {
    let config = ConfigStore::load()?;
    if config.mount_dir.is_empty() && !config.mount_configured {
        return Ok(()); // 已是初始态，无需重置
    }
    let reset = config.with(
        None,
        None,
        Some(String::new()),
        Some(false),
        None,
        None,
        None,
        None,
        None,
        None,
    );
    ConfigStore::save(&reset)
}

/// 取消正在等待本地回调的 OAuth 授权流程。
#[tauri::command]
pub async fn auth_cancel_login() -> AppResult<()> {
    AUTH_SERVICE.cancel_authorize().await;
    Ok(())
}

/// 停止同步运行时，清理当前账号的同步数据与目录配置，然后删除登录 token。
#[tauri::command]
pub async fn auth_logout() -> AppResult<()> {
    // 清空当前账号同步状态
    drop_runtime_async().await;
    clear_account_caches();
    let _ = reset_account_config();
    AUTH_SERVICE.logout().await
}

/// 使用当前认证信息读取账号资料。
#[tauri::command]
pub async fn auth_get_user_info() -> AppResult<UserInfo> {
    let api = UserInfoApi::new(AUTH_SERVICE.clone());
    api.get().await
}

/// 以本地 token store 是否存在有效记录判断登录状态。
#[tauri::command]
pub async fn auth_is_logged_in() -> AppResult<bool> {
    use crate::auth::token_store::global_store;
    Ok(global_store().load()?.is_some())
}
