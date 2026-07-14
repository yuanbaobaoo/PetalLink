//! Files API 的 POST 创建与 PATCH 更新、移动、软删除操作。

use serde_json::Value;

use super::request::{
    build_create_folder_body, canonical_parent_id, delete_path, file_path, update_path,
    validate_file_id, validate_file_id_value,
};
use super::response::{
    parse_verified_written_drive_file, protocol_error, require_official_write_ok, single_parent,
    verify_created_folder, verify_file_id, verify_parent, verify_written_file_id,
    verify_written_parent, write_protocol_error,
};
use super::FilesApi;
use crate::drive::ascii_json::ascii_json_encode;
use crate::drive::client::{parse_json_response, parse_json_response_with_semantics};
use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult, RequestSemantics};

impl FilesApi {
    /// POST 创建文件夹。对齐 dart `FilesApi.createFolder({name, parentId?})`。
    ///
    /// 这是非幂等 POST，因此必须先在目标父目录内查重；写请求失败后也必须再次按
    /// `parentFolder + fileName` 唯一核验。唯一匹配视为已经提交，零匹配把原错误返回给
    /// 调用方决定何时重试，多匹配或核验失败则拒绝再次 POST。
    pub async fn create_folder(&self, name: &str, parent_id: Option<&str>) -> AppResult<DriveFile> {
        if name.trim().is_empty() {
            return Err(AppError::generic("文件夹名称不能为空"));
        }
        let expected_parent = canonical_parent_id(parent_id)?;

        if let Some(existing) = self
            .find_unique_folder_in_parent(name, expected_parent)
            .await?
        {
            tracing::info!(
                folder_id = %existing.id,
                folder_name = name,
                parent_id = expected_parent,
                "创建文件夹前核验命中唯一同名目录，跳过 POST"
            );
            return Ok(existing);
        }

        let submitted = self
            .create_folder_once(name, parent_id, expected_parent)
            .await;
        match submitted {
            Ok(file) => Ok(file),
            Err(submit_error) => {
                match self
                    .find_unique_folder_in_parent(name, expected_parent)
                    .await
                {
                    Ok(Some(existing)) => {
                        tracing::info!(
                            folder_id = %existing.id,
                            folder_name = name,
                            parent_id = expected_parent,
                            error = %submit_error,
                            "创建文件夹响应不确定，父目录唯一核验确认已提交"
                        );
                        Ok(existing)
                    }
                    // 只有明确的零匹配才把原错误交还调用方，允许稍后显式重试。
                    Ok(None) => Err(submit_error),
                    Err(verification_error) => Err(AppError::generic(format!(
                        "创建文件夹结果不确定：{submit_error}；父目录唯一核验失败：{verification_error}"
                    ))),
                }
            }
        }
    }

    /// 提交一次非幂等目录创建，并严格核验 200 File 响应。
    async fn create_folder_once(
        &self,
        name: &str,
        parent_id: Option<&str>,
        expected_parent: &str,
    ) -> AppResult<DriveFile> {
        let body = build_create_folder_body(name, parent_id);
        let encoded = ascii_json_encode(&body);
        let path = "/files?fields=*";
        let resp = self
            .send_post(path, encoded.into_bytes(), "application/json")
            .await?;
        let auth_already_replayed = require_official_write_ok(&resp, "createFolder")?;
        let body_json: Value =
            parse_json_response_with_semantics(resp, "createFolder", RequestSemantics::Write)
                .await?;
        let file =
            parse_verified_written_drive_file(&body_json, "createFolder", auth_already_replayed)?;
        verify_created_folder(
            &file,
            name,
            expected_parent,
            RequestSemantics::Write,
            auth_already_replayed,
        )?;
        Ok(file)
    }

    /// 在指定父目录中查找唯一同名目录，多匹配时返回歧义错误。
    async fn find_unique_folder_in_parent(
        &self,
        name: &str,
        expected_parent: &str,
    ) -> AppResult<Option<DriveFile>> {
        let request_parent = (expected_parent != "root").then_some(expected_parent);
        let listed = self.list_all(request_parent).await?;
        let mut matches = Vec::new();
        for file in listed {
            if file.name != name || !file.is_folder() {
                continue;
            }
            verify_created_folder(&file, name, expected_parent, RequestSemantics::Read, false)?;
            matches.push(file);
        }
        match matches.len() {
            0 => Ok(None),
            1 => Ok(matches.pop()),
            count => Err(AppError::generic(format!(
                "父目录 {expected_parent} 中存在 {count} 个同名文件夹，创建结果有歧义"
            ))),
        }
    }

    /// 删除文件（软删除，移入回收站"最近删除"）。
    ///
    /// **重要**：华为 Drive API 的 `DELETE /drive/v1/files/{id}` 是**永久删除**，不进回收站。
    /// 要实现软删除（进"最近删除"），必须用 PATCH 更新 `recycled: true`。
    /// 对齐华为官方文档 Files:update → recycled 字段。
    pub async fn delete(&self, id: &str) -> AppResult<()> {
        self.delete_verified(id).await.map(|_| ())
    }

    /// 软删除并返回已经核验的 File 响应。
    ///
    /// 华为 Files:update 的软删除成功合同是 `200 + File JSON`。只有响应资源仍是同一个
    /// fileId 且明确返回 `recycled=true` 才能驱动后续本地删除和成功结算。
    pub async fn delete_verified(&self, id: &str) -> AppResult<DriveFile> {
        validate_file_id(id)?;
        let path = delete_path(id);
        let mut body = serde_json::Map::new();
        body.insert("recycled".into(), Value::Bool(true));
        let encoded = ascii_json_encode(&Value::Object(body));
        let resp = self
            .client
            .patch(&path, encoded.into_bytes(), "application/json")
            .await?;
        let auth_already_replayed = require_official_write_ok(&resp, "delete")?;
        let body_json: Value =
            parse_json_response_with_semantics(resp, "delete", RequestSemantics::Write).await?;
        let file = parse_verified_written_drive_file(&body_json, "delete", auth_already_replayed)?;
        verify_written_file_id(&file, id, "delete", auth_already_replayed)?;
        if body_json.get("recycled") != Some(&Value::Bool(true)) {
            return Err(write_protocol_error(
                "delete",
                auth_already_replayed,
                "响应未明确确认 recycled=true",
            ));
        }
        Ok(file)
    }

    /// 通过稳定 fileId 核验不确定的删除结果。
    pub async fn verify_deleted(&self, id: &str) -> AppResult<bool> {
        validate_file_id(id)?;
        let path = format!("{}?fields=*", file_path(id));
        let response = match self.client.get(&path).await {
            Ok(response) => response,
            Err(error) if error.drive_status() == Some(404) => return Ok(true),
            Err(error) => return Err(error),
        };
        let body: Value = parse_json_response(response, "verify delete").await?;
        let file = parse_verified_written_drive_file(&body, "verify delete", false)?;
        verify_file_id(&file, id, "verify delete", RequestSemantics::Read, false)?;
        match body.get("recycled") {
            Some(Value::Bool(recycled)) => Ok(*recycled),
            _ => Err(protocol_error(
                "verify delete",
                RequestSemantics::Read,
                false,
                "响应缺少明确 recycled 布尔值",
            )),
        }
    }

    /// 更新文件（重命名/移动/改描述）。
    /// 对齐 dart `FilesApi.update(id, {newName?, newParentFolder?, description?})`。
    ///
    /// 关键：body 用 [`ascii_json_encode`] 编码。
    pub async fn update(
        &self,
        id: &str,
        new_name: Option<&str>,
        new_parent_folder: Option<&str>,
        description: Option<&str>,
    ) -> AppResult<DriveFile> {
        validate_file_id(id)?;
        if let Some(target_parent) = new_parent_folder {
            validate_file_id_value(target_parent, "目标 parentFolder")?;
            // Files:update 移动必须同时提交旧、新 parent。先读当前 parent 也让重复调用具备
            // fileId 级幂等性：若响应曾丢失但移动已经提交，则不再次发送移动 PATCH。
            let current = self.get(id).await?;
            verify_file_id(
                &current,
                id,
                "move preflight",
                RequestSemantics::Read,
                false,
            )?;
            let current_parent =
                single_parent(&current, "move preflight", RequestSemantics::Read, false)?;
            if current_parent == target_parent {
                if new_name.is_none() && description.is_none() {
                    return Ok(current);
                }
                return self.update_verified(id, new_name, None, description).await;
            }
            return self
                .update_verified(
                    id,
                    new_name,
                    Some((current_parent, target_parent)),
                    description,
                )
                .await;
        }
        self.update_verified(id, new_name, None, description).await
    }

    /// 使用官方成对 parent query 参数移动文件，并核验响应仍是同一个 fileId 且目标父目录
    /// 已生效。调用方已经持有可信旧 parent 时可直接使用，避免额外 GET。
    pub async fn move_file(
        &self,
        id: &str,
        old_parent_folder: &str,
        new_parent_folder: &str,
    ) -> AppResult<DriveFile> {
        validate_file_id(id)?;
        validate_file_id_value(old_parent_folder, "旧 parentFolder")?;
        validate_file_id_value(new_parent_folder, "目标 parentFolder")?;
        if old_parent_folder == new_parent_folder {
            let current = self.get(id).await?;
            verify_file_id(&current, id, "move", RequestSemantics::Read, false)?;
            verify_parent(
                &current,
                new_parent_folder,
                "move",
                RequestSemantics::Read,
                false,
            )?;
            return Ok(current);
        }
        self.update_verified(id, None, Some((old_parent_folder, new_parent_folder)), None)
            .await
    }

    /// 重命名并核验 Huawei 返回的 File 身份和最终名称。
    pub async fn rename_file(&self, id: &str, new_name: &str) -> AppResult<DriveFile> {
        self.update(id, Some(new_name), None, None).await
    }

    /// 提交一次更新，并核验文件身份及请求指定的名称或父目录。
    async fn update_verified(
        &self,
        id: &str,
        new_name: Option<&str>,
        move_parents: Option<(&str, &str)>,
        description: Option<&str>,
    ) -> AppResult<DriveFile> {
        let mut body = serde_json::Map::new();
        if let Some(name) = new_name {
            body.insert("fileName".into(), Value::String(name.to_string()));
        }
        if let Some(desc) = description {
            body.insert("description".into(), Value::String(desc.to_string()));
        }
        let encoded = ascii_json_encode(&Value::Object(body));
        let path = update_path(id, move_parents);
        let resp = self
            .send_patch(&path, encoded.into_bytes(), "application/json")
            .await?;
        let auth_already_replayed = require_official_write_ok(&resp, "update")?;
        let body_json: Value =
            parse_json_response_with_semantics(resp, "update", RequestSemantics::Write).await?;
        let file = parse_verified_written_drive_file(&body_json, "update", auth_already_replayed)?;
        verify_written_file_id(&file, id, "update", auth_already_replayed)?;
        if let Some(expected_name) = new_name {
            if file.name != expected_name {
                return Err(write_protocol_error(
                    "rename",
                    auth_already_replayed,
                    "响应 fileName 与目标名称不一致",
                ));
            }
        }
        if let Some((_, target_parent)) = move_parents {
            verify_written_parent(&file, target_parent, "move", auth_already_replayed)?;
        }
        Ok(file)
    }

    /// 发送带 body 的 POST，并沿用客户端的严格成功与认证重放规则。
    async fn send_post(
        &self,
        path: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> AppResult<reqwest::Response> {
        self.client.post(path, Some(body), content_type).await
    }

    /// 发送 PATCH，并让客户端保留可能已提交的结构化失败语义。
    async fn send_patch(
        &self,
        path: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> AppResult<reqwest::Response> {
        self.client.patch(path, body, content_type).await
    }
}
