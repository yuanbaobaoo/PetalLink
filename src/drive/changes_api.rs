//! Changes API —— 华为 Drive 增量变更接口（GET /drive/v1/changes）。
//!
//! 用于自动云端刷新的增量路径：相比全量 BFS（refresh_cloud_tree）大幅省流量、提速。
//! cursor 持久化后可跨重启复用；失效或接口异常时由调用方回退全量 BFS。
//!
//! ⚠️ 字段名（changes/nextCursor/removed）基于 GDrive 协议推断 + 阶段二验证。
//!    若华为实际字段不同，调整 ChangeListResult::from_json 的键名探测。

use std::sync::Arc;

use crate::drive::client::{handle_error_response, DriveClient};
use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult};

/// 变更类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    /// 文件被移除（云端删除）。具体判定见 Change::from_json 注释。
    Removed,
    /// 文件新增或元数据修改（含内容更新、改名、移动）。
    Modified,
}

/// 单条变更：一个云端文件的增/改/删事件。
#[derive(Debug, Clone)]
pub struct Change {
    pub kind: ChangeKind,
    pub file: DriveFile,
}

/// 变更列表 + 分页游标。
#[derive(Debug, Clone)]
pub struct ChangeListResult {
    pub changes: Vec<Change>,
    /// 下一页游标；None 表示已追平最新（无更多变更）。
    pub next_cursor: Option<String>,
}

impl ChangeListResult {
    /// 从 JSON 解析。键名容错：nextCursor 优先，回退 cursor（对齐 FileListResult 惯例）。
    /// ⚠️ 字段名以阶段二验证报告为准，必要时调整。
    pub fn from_json(json: &serde_json::Value) -> Self {
        let changes = json
            .get("changes")
            .or_else(|| json.get("items")) // 回退名
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(Change::from_json).collect())
            .unwrap_or_default();

        let next_cursor = json
            .get("nextCursor")
            .or_else(|| json.get("newStartCursor"))
            .or_else(|| json.get("cursor"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        Self { changes, next_cursor }
    }
}

impl Change {
    /// 从单条 change JSON 解析。
    /// ⚠️ removed 判定以阶段二验证为准：GDrive 用 removed:true，华为可能用 fileDeleted 或其他。
    pub fn from_json(v: &serde_json::Value) -> Option<Self> {
        // 删除判定：优先看显式标志，再看是否缺 file 元数据
        let is_removed = v
            .get("removed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || v.get("fileDeleted").and_then(|v| v.as_bool()).unwrap_or(false);

        let file = if is_removed {
            // 删除事件可能只带 fileId，构造最小 DriveFile（id 来自 fileId 字段）
            let id = v.get("fileId").and_then(|v| v.as_str())
                .or_else(|| v.get("id").and_then(|v| v.as_str()))?
                .to_string();
            DriveFile {
                id,
                name: String::new(),
                category: crate::drive::models::FileCategory::None,
                size: 0,
                parent_folder: None,
                description: None,
                created_time: None,
                edited_time: None,
                mime_type: None,
                content_hash: None,
                thumbnail_link: None,
            }
        } else {
            // 增/改事件：file 字段内是完整 DriveFile
            // 注意 DriveFile::from_json 返回 Option；解析失败则整条 change 返回 None（被 filter_map 过滤）
            let file_json = v.get("file").unwrap_or(v);
            DriveFile::from_json(file_json)?
        };

        Some(Self {
            kind: if is_removed { ChangeKind::Removed } else { ChangeKind::Modified },
            file,
        })
    }
}

pub struct ChangesApi {
    client: Arc<DriveClient>,
}

impl ChangesApi {
    pub fn new(client: Arc<DriveClient>) -> Self {
        Self { client }
    }

    /// 拉取一页增量变更（pageSize 默认 100）。
    pub async fn list_changes(&self, cursor: Option<&str>) -> AppResult<ChangeListResult> {
        let mut url = format!("/drive/v1/changes?fields=*&pageSize=100");
        if let Some(c) = cursor {
            if !c.is_empty() {
                url.push_str(&format!("&cursor={}", crate::drive::files_api::urlencoding(c)));
            }
        }
        let resp = self.client.get(&url).await?;
        if !resp.status().is_success() {
            return Err(handle_error_response(resp).await);
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::generic(format!("解析 changes 响应失败：{e}")))?;
        Ok(ChangeListResult::from_json(&body))
    }

    /// 拉取全部增量变更（自动分页至 next_cursor 为空）。最多 100 页兜底。
    pub async fn list_all_changes(&self, start_cursor: Option<&str>) -> AppResult<(Vec<Change>, Option<String>)> {
        const MAX_PAGES: usize = 100;
        let mut all = Vec::new();
        let mut cursor: Option<String> = start_cursor.map(|s| s.to_string());
        for _ in 0..MAX_PAGES {
            let result = self.list_changes(cursor.as_deref()).await?;
            all.extend(result.changes);
            cursor = result.next_cursor;
            if cursor.is_none() {
                return Ok((all, None));
            }
        }
        tracing::warn!("list_all_changes 超过 {MAX_PAGES} 页，截断；返回最后 cursor");
        Ok((all, cursor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_modified_change() {
        // 增/改事件：file 字段内是完整文件
        let json = serde_json::json!({
            "changes": [{
                "file": { "id": "f1", "fileName": "a.txt", "mimeType": "text/plain", "size": 100 }
            }],
            "nextCursor": "cur123"
        });
        let r = ChangeListResult::from_json(&json);
        assert_eq!(r.changes.len(), 1);
        assert_eq!(r.changes[0].kind, ChangeKind::Modified);
        assert_eq!(r.changes[0].file.name, "a.txt");
        assert_eq!(r.next_cursor.as_deref(), Some("cur123"));
    }

    #[test]
    fn test_parse_removed_change() {
        // 删除事件：removed 标志 + fileId
        let json = serde_json::json!({
            "changes": [{ "removed": true, "fileId": "f9" }]
        });
        let r = ChangeListResult::from_json(&json);
        assert_eq!(r.changes.len(), 1);
        assert_eq!(r.changes[0].kind, ChangeKind::Removed);
        assert_eq!(r.changes[0].file.id, "f9");
        assert!(r.next_cursor.is_none());
    }

    #[test]
    fn test_parse_empty() {
        let json = serde_json::json!({ "changes": [] });
        let r = ChangeListResult::from_json(&json);
        assert!(r.changes.is_empty());
        assert!(r.next_cursor.is_none());
    }
}
