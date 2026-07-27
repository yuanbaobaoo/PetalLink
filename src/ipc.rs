//! Tauri IPC 合同与 TypeScript bindings 生成。
//!
//! 此处的 command 集合同时用于运行时注册和前端代码生成，避免维护两份命令清单。

use std::{fs, path::PathBuf};

use serde::Serialize;
use specta::Type;
use specta_typescript::Typescript;
use tauri::Wry;
use tauri_specta::{collect_commands, collect_events, Builder, ErrorHandlingMode, Event};

use crate::commands;
use crate::data::repository;
use crate::sync::state::SyncGlobalState;
use crate::sync::transfer_state::{TransferErrorKind, TransferOperation};

/// 完整同步状态快照事件。
#[derive(Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "sync_state")]
pub struct SyncStateEvent(
    /// 当前完整同步状态。
    pub SyncGlobalState,
);

/// 云盘目录内容已变化，前端应重新加载当前目录。
#[derive(Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "folder_content_changed")]
pub struct FolderContentChangedEvent;

/// 传输队列已变化，前端应重新加载队列。
#[derive(Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "transfer_update")]
pub struct TransferUpdateEvent;

/// 后台自动上传失败事件。
#[derive(Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "upload_failed")]
pub struct UploadFailedEvent {
    /// 相对挂载目录的文件路径。
    pub rel_path: String,
    /// 上传失败的文件名。
    pub name: String,
    /// 面向用户的失败原因。
    pub error: String,
}

/// 目录递归同步进度事件。
#[derive(Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "folder_sync_progress")]
pub struct FolderSyncProgressEvent {
    /// 已完成任务数。
    pub done: usize,
    /// 本轮任务总数。
    pub total: usize,
}

/// 原生菜单请求打开设置页。
#[derive(Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "navigate_settings")]
pub struct NavigateSettingsEvent;

/// 传输方向持久化值，供前端生成同源常量。
#[derive(Serialize, Type)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
struct TransferDirectionConstants {
    upload: i32,
    download: i32,
    delete: i32,
    download_update: i32,
}

/// 传输状态持久化值，供前端生成同源常量。
#[derive(Serialize, Type)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
struct TransferStateConstants {
    pending: i32,
    running: i32,
    waiting_for_network: i32,
    backing_off: i32,
    verifying_remote: i32,
    restart_required: i32,
    completed: i32,
    failed: i32,
    canceled: i32,
}

/// 传输操作持久化值，供前端生成同源常量。
#[derive(Serialize, Type)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
struct TransferOperationConstants {
    create: i32,
    update: i32,
    download: i32,
    download_update: i32,
    delete: i32,
    r#move: i32,
    rename: i32,
    create_folder: i32,
}

/// 传输错误分类持久化值，供前端生成同源常量。
#[derive(Serialize, Type)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
struct TransferErrorKindConstants {
    network: i32,
    timeout: i32,
    auth: i32,
    rate_limit: i32,
    server: i32,
    quota: i32,
    permission: i32,
    validation: i32,
    session_expired: i32,
    remote_ambiguous: i32,
    local_changed: i32,
    unknown: i32,
}

/// 前端自动生成 bindings 的固定位置。
pub fn bindings_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("app/api/generated.ts")
}

/// 构造完整 IPC 注册表。
pub fn builder() -> Builder<Wry> {
    Builder::<Wry>::new()
        .commands(collect_commands![
            commands::auth_check_secret,
            commands::auth_restore,
            commands::auth_login,
            commands::auth_cancel_login,
            commands::auth_logout,
            commands::auth_get_user_info,
            commands::auth_is_logged_in,
            commands::drive_list,
            commands::drive_list_all,
            commands::drive_get_file,
            commands::drive_create_folder,
            commands::drive_delete_file,
            commands::drive_rename_file,
            commands::drive_move_file,
            commands::drive_search,
            commands::drive_get_thumbnail,
            commands::drive_get_about,
            commands::drive_download_file,
            commands::drive_upload_file,
            commands::sync_manual_refresh,
            commands::sync_check_safe_free_up,
            commands::sync_check_file_local_status,
            commands::sync_batch_file_status,
            commands::sync_free_up_space,
            commands::sync_free_up_batch,
            commands::sync_list_freeable_in_folder,
            commands::sync_download_on_demand,
            commands::sync_folder_recursive,
            commands::sync_retry_failed,
            commands::sync_state,
            commands::sync_items_by_folder,
            commands::config_load,
            commands::config_save,
            commands::tray_set_visible,
            commands::config_export_json,
            commands::config_import_json,
            commands::transfer_list_all,
            commands::transfer_has_active,
            commands::transfer_clear_completed,
            commands::transfer_clear_failed,
            commands::transfer_clear_finished,
            commands::transfer_retry,
            commands::open_in_finder,
            commands::launch_at_login_is_enabled,
            commands::launch_at_login_set_enabled,
            commands::tray_is_visible,
            commands::app_clear_cache,
            commands::logs_list,
            commands::logs_export,
            commands::logs_clear,
            commands::app_get_version,
        ])
        .events(collect_events![
            SyncStateEvent,
            FolderContentChangedEvent,
            TransferUpdateEvent,
            UploadFailedEvent,
            FolderSyncProgressEvent,
            NavigateSettingsEvent,
        ])
        // 保持当前前端语义：成功 resolve，AppError 通过 Promise rejection 抛出。
        .error_handling(ErrorHandlingMode::Throw)
        .constant(
            "DELETE_TRACE_ERROR_PREFIX",
            commands::drive::DELETE_TRACE_ERROR_PREFIX,
        )
        .constant(
            "SYNC_USER_MESSAGE_RULES",
            crate::sync::user_messages::ipc_user_message_rules(),
        )
        .constant(
            "TRANSFER_DIR",
            TransferDirectionConstants {
                upload: repository::transfer_direction::UPLOAD,
                download: repository::transfer_direction::DOWNLOAD,
                delete: repository::transfer_direction::DELETE,
                download_update: repository::transfer_direction::DOWNLOAD_UPDATE,
            },
        )
        .constant(
            "TRANSFER_STATE",
            TransferStateConstants {
                pending: repository::transfer_state::PENDING,
                running: repository::transfer_state::RUNNING,
                waiting_for_network: repository::transfer_state::WAITING_FOR_NETWORK,
                backing_off: repository::transfer_state::BACKING_OFF,
                verifying_remote: repository::transfer_state::VERIFYING_REMOTE,
                restart_required: repository::transfer_state::RESTART_REQUIRED,
                completed: repository::transfer_state::COMPLETED,
                failed: repository::transfer_state::FAILED,
                canceled: repository::transfer_state::CANCELED,
            },
        )
        .constant(
            "TRANSFER_OPERATION",
            TransferOperationConstants {
                create: TransferOperation::Create as i32,
                update: TransferOperation::Update as i32,
                download: TransferOperation::Download as i32,
                download_update: TransferOperation::DownloadUpdate as i32,
                delete: TransferOperation::Delete as i32,
                r#move: TransferOperation::Move as i32,
                rename: TransferOperation::Rename as i32,
                create_folder: TransferOperation::CreateFolder as i32,
            },
        )
        .constant(
            "TRANSFER_ERROR_KIND",
            TransferErrorKindConstants {
                network: TransferErrorKind::Network as i32,
                timeout: TransferErrorKind::Timeout as i32,
                auth: TransferErrorKind::Auth as i32,
                rate_limit: TransferErrorKind::RateLimit as i32,
                server: TransferErrorKind::Server as i32,
                quota: TransferErrorKind::Quota as i32,
                permission: TransferErrorKind::Permission as i32,
                validation: TransferErrorKind::Validation as i32,
                session_expired: TransferErrorKind::SessionExpired as i32,
                remote_ambiguous: TransferErrorKind::RemoteAmbiguous as i32,
                local_changed: TransferErrorKind::LocalChanged as i32,
                unknown: TransferErrorKind::Unknown as i32,
            },
        )
        // 现有 IPC 使用 number 承载文件大小、时间戳和 SQLite ID。
        // 这些值都有业务范围约束，不会接近 JS 的 2^53 安全整数上限。
        .dangerously_cast_bigints_to_number()
}

/// 将 Rust IPC 合同导出为前端 TypeScript。
pub fn export_bindings() {
    let path = bindings_path();
    builder()
        .export(Typescript::default(), &path)
        .expect("生成 TypeScript IPC bindings 失败");
    let generated = fs::read_to_string(&path).expect("读取生成的 TypeScript IPC bindings 失败");
    fs::write(&path, normalize_generated_comments(&generated))
        .expect("规范化 TypeScript IPC bindings 注释失败");
}

/// 将生成器输出的 JSDoc 转为项目统一使用的行注释。
fn normalize_generated_comments(source: &str) -> String {
    let source = source.replace(
        "import { invoke as __TAURI_INVOKE } from \"@tauri-apps/api/core\";",
        "import { invoke as __TAURI_INVOKE } from \"./tauri\";",
    );
    let mut output = String::with_capacity(source.len());
    let mut doc_indent = "";
    let mut in_doc = false;

    for line in source.lines() {
        let trimmed = line.trim_start();
        let indent = &line[..line.len() - trimmed.len()];

        if in_doc {
            if trimmed == "*/" {
                in_doc = false;
                continue;
            }
            let content = trimmed.strip_prefix('*').unwrap_or(trimmed).trim();
            if let Some(content) = content.strip_suffix("*/") {
                push_generated_comment(&mut output, doc_indent, content.trim());
                in_doc = false;
            } else {
                push_generated_comment(&mut output, doc_indent, content);
            }
            continue;
        }

        if let Some(content) = trimmed.strip_prefix("/**") {
            doc_indent = indent;
            if let Some(content) = content.strip_suffix("*/") {
                push_generated_comment(&mut output, indent, content.trim());
            } else {
                push_generated_comment(&mut output, indent, content.trim());
                in_doc = true;
            }
            continue;
        }

        let normalized = match trimmed {
            "// This file has been generated by Tauri Specta. Do not edit this file manually." => {
                "// 此文件由 Tauri Specta 自动生成，禁止手动修改。"
            }
            "/* Constants */" => "// 常量",
            "/* Types */" => "// 类型",
            "/* Tauri Specta runtime */" => "// Tauri Specta 事件运行时",
            _ => line,
        };
        output.push_str(normalized.trim_end());
        output.push('\n');
    }

    // 固定为单个 EOF 换行，避免生成器版本差异产生无意义 diff。
    while output.ends_with("\n\n") {
        output.pop();
    }
    output
}

/// 写入非空生成注释，并翻译固定的分区标题。
fn push_generated_comment(output: &mut String, indent: &str, content: &str) {
    if content.is_empty() {
        return;
    }
    let content = match content {
        "Commands" => "命令",
        "Events" => "事件",
        value => value,
    };
    output.push_str(indent);
    output.push_str("// ");
    output.push_str(content);
    output.push('\n');
}

/// IPC bindings 生成合同测试。
#[cfg(test)]
mod tests {
    /// 验证 bindings 可生成，且不会重新引入单行 JSDoc。
    #[test]
    fn export_typescript_bindings() {
        super::export_bindings();
        let generated =
            std::fs::read_to_string(super::bindings_path()).expect("读取生成 bindings 失败");
        assert!(!generated.contains("/**"));
        assert!(generated.contains("import { invoke as __TAURI_INVOKE } from \"./tauri\";"));
    }
}
