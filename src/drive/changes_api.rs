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

/// 变更类型。判定依据：华为 change 事件的 `changeType` 字段（真机验证）。
/// 已知值：`trashDone`（移入回收站/软删除）。其余（create/update/untrash 等）按非删除处理。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    /// 文件被移入回收站（软删除）。changeType == "trashDone" 或 file.recycled == true。
    Removed,
    /// 文件新增或元数据修改（含内容更新、改名、移动、从回收站恢复等）。
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
    /// 从 JSON 解析。已校准（阶段二真机验证）：
    /// - 数组字段：`changes`（华为确认）
    /// - 分页游标字段：`newStartCursor`（华为确认，**非** GDrive 的 nextCursor）
    pub fn from_json(json: &serde_json::Value) -> Self {
        let changes = json
            .get("changes")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(Change::from_json).collect())
            .unwrap_or_default();

        // 华为用 newStartCursor；保留 nextCursor 回退以防接口变体
        let next_cursor = json
            .get("newStartCursor")
            .or_else(|| json.get("nextCursor"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        Self { changes, next_cursor }
    }
}

impl Change {
    /// 从单条 change JSON 解析。已校准（阶段二真机验证，2026-07-06）：
    ///
    /// 华为 change 事件结构（与 GDrive 差异显著）：
    /// ```json
    /// { "category":"drive#change", "changeType":"trashDone", "deleted":false,
    ///   "file":{...完整 DriveFile，删除事件也带...}, "fileId":"...", "type":"File" }
    /// ```
    /// - **删除判定**：`changeType == "trashDone"`（移入回收站）。**非** GDrive 的 `removed:true`。
    ///   注意 `deleted` 字段恒为 false（华为用 changeType 区分，不用 deleted）。
    /// - **file 字段**：所有事件都带完整 file（删除事件 file.recycled==true）。
    /// - 删除事件也带完整 file，直接解析即可，无需构造最小 DriveFile。
    pub fn from_json(v: &serde_json::Value) -> Option<Self> {
        // 删除判定：changeType == "trashDone" 为主，file.recycled 兜底
        let is_removed = v.get("changeType").and_then(|v| v.as_str()) == Some("trashDone")
            || v.get("file").and_then(|f| f.get("recycled")).and_then(|v| v.as_bool()).unwrap_or(false);

        // file 字段：所有事件都带完整 file；解析失败则用 fileId 构造最小 DriveFile 兜底
        let file = v.get("file")
            .and_then(DriveFile::from_json)
            .or_else(|| {
                // 极端兜底：file 缺失或解析失败，用顶层 fileId 构造最小 DriveFile
                v.get("fileId").and_then(|v| v.as_str()).map(|id| DriveFile {
                    id: id.to_string(),
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
                })
            })?;

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

    /// 获取初始游标（startCursor）。GET /changes/getStartCursor。
    ///
    /// 华为的 /changes 接口强制要求 cursor，无 cursor 直接 400；初始 cursor 必须先通过本端点获取。
    /// 响应：`{"category":"drive#startCursor","startCursor":"311296"}`
    pub async fn get_start_cursor(&self) -> AppResult<String> {
        let resp = self.client.get("/changes/getStartCursor").await?;
        if !resp.status().is_success() {
            return Err(handle_error_response(resp).await);
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::generic(format!("解析 startCursor 响应失败：{e}")))?;
        body.get("startCursor")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| AppError::generic("startCursor 响应缺少 startCursor 字段".to_string()))
    }

    /// 拉取一页增量变更（pageSize 默认 100）。
    /// 路径相对 base_url（base_url 已含 /drive/v1），对齐 about_api 的 /about 写法。
    pub async fn list_changes(&self, cursor: Option<&str>) -> AppResult<ChangeListResult> {
        let mut url = format!("/changes?fields=*&pageSize=100");
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

    /// 拉取全部增量变更（自动分页至追平最新状态）。
    ///
    /// 华为 API 的 newStartCursor 字段即使已追平也会返回非空值（类似 GDrive 的
    /// nextPageToken——是"下次轮询的起点"而非"还有更多数据"的标记）。
    /// 因此不能仅靠 cursor.is_none() 判断结束，需结合页内条目数：返回 0 条即已追平。
    pub async fn list_all_changes(&self, start_cursor: Option<&str>) -> AppResult<(Vec<Change>, Option<String>)> {
        let mut all = Vec::new();
        let mut cursor: Option<String> = start_cursor.map(|s| s.to_string());
        let mut pages = 0u32;
        loop {
            let result = self.list_changes(cursor.as_deref()).await?;
            let page_count = result.changes.len();
            all.extend(result.changes);
            cursor = result.next_cursor;
            pages += 1;
            // 追平判定：若无新条目，或 cursor 为空，视为已追上
            if page_count == 0 || cursor.is_none() {
                tracing::info!(total = all.len(), pages, "list_all_changes 已追平最新状态");
                return Ok((all, None));
            }
            tracing::debug!(page_total = all.len(), last_page = page_count, pages, "list_all_changes 继续翻页…");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_modified_change() {
        // 增/改事件（校准自真机）：changeType 非 trashDone，file 字段内是完整文件，游标用 newStartCursor
        let json = serde_json::json!({
            "category": "drive#changeList",
            "changes": [{
                "category": "drive#change",
                "changeType": "update",
                "deleted": false,
                "file": { "id": "f1", "fileName": "a.txt", "mimeType": "text/plain", "size": 100 },
                "fileId": "f1",
                "type": "File"
            }],
            "newStartCursor": "311298"
        });
        let r = ChangeListResult::from_json(&json);
        assert_eq!(r.changes.len(), 1);
        assert_eq!(r.changes[0].kind, ChangeKind::Modified);
        assert_eq!(r.changes[0].file.name, "a.txt");
        assert_eq!(r.next_cursor.as_deref(), Some("311298"));
    }

    #[test]
    fn test_parse_removed_change() {
        // 删除事件（校准自真机）：changeType=="trashDone"，file 字段仍带完整文件（recycled:true）
        let json = serde_json::json!({
            "category": "drive#changeList",
            "changes": [{
                "category": "drive#change",
                "changeType": "trashDone",
                "deleted": false,
                "file": { "id": "f9", "fileName": "del.txt", "mimeType": "text/plain", "size": 10, "recycled": true },
                "fileId": "f9",
                "type": "File"
            }],
            "newStartCursor": "311299"
        });
        let r = ChangeListResult::from_json(&json);
        assert_eq!(r.changes.len(), 1);
        assert_eq!(r.changes[0].kind, ChangeKind::Removed);
        assert_eq!(r.changes[0].file.id, "f9");
        assert_eq!(r.changes[0].file.name, "del.txt");
        assert_eq!(r.next_cursor.as_deref(), Some("311299"));
    }

    #[test]
    fn test_parse_empty() {
        // 空变更（校准自真机）：changes 空数组 + newStartCursor 与请求 cursor 相同
        let json = serde_json::json!({
            "category": "drive#changeList",
            "changes": [],
            "newStartCursor": "311296"
        });
        let r = ChangeListResult::from_json(&json);
        assert!(r.changes.is_empty());
        assert_eq!(r.next_cursor.as_deref(), Some("311296"));
    }
}
