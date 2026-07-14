//! Files API —— GET 列举/搜索、POST 创建、PATCH 更新/软删除。
//!
//! 对齐 `legacy/lib/drive/api/files_api.dart`。
//!
//! # 华为 API 怪癖
//! - **parentFolder 查询语法**：不用 parentFolder 参数，而用 `queryParam='root' in parentFolder`
//!   （单引号包裹 token）。列出根目录用 `'root'`，列出子目录用 `'<id>'`。
//! - **asciiJsonEncode**：createFolder / update 的 application/json 请求体必须用 ASCII-only 编码，
//!   否则中文名报 400 `21004002 fileName can not be blank`。
//! - **createFolder**：mimeType 必填，root 目录省略 parentFolder。

/// 文件读取与严格分页实现。
mod read;
/// 文件请求参数校验与编码。
mod request;
/// 文件响应协议解析与写后核验。
mod response;
/// 文件创建、更新和软删除实现。
mod write;

use std::sync::Arc;

use crate::error::{AppError, AppResult};

pub use request::{build_create_folder_body, urlencoding};

/// 生产环境目录全量分页的最大请求页数。
const PRODUCTION_MAX_PAGES: usize = 1_000;

/// Files:list 的客户端分页上限。
///
/// 华为只定义单页大小上限，没有定义目录总页数。客户端仍需要有限上限来避免服务端
/// cursor 循环或异常数据导致永久索引；达到上限且仍有下一页时必须失败，不能返回部分树。
#[derive(Debug, Clone, Copy)]
pub struct PaginationPolicy {
    max_pages: usize,
}

impl PaginationPolicy {
    /// 创建非零分页上限，否则返回配置错误。
    pub fn new(max_pages: usize) -> AppResult<Self> {
        if max_pages == 0 {
            return Err(AppError::generic("Files 分页上限必须大于 0"));
        }
        Ok(Self { max_pages })
    }

    /// 返回生产环境分页策略。
    const fn production() -> Self {
        Self {
            max_pages: PRODUCTION_MAX_PAGES,
        }
    }
}

/// 提供严格分页、解析及写后核验的云盘文件接口。
pub struct FilesApi {
    client: Arc<crate::drive::client::DriveClient>,
    pagination: PaginationPolicy,
}

impl FilesApi {
    /// 使用生产分页上限创建文件接口。
    pub fn new(client: Arc<crate::drive::client::DriveClient>) -> Self {
        Self {
            client,
            pagination: PaginationPolicy::production(),
        }
    }

    /// 使用可控的分页上限构造真实 Files API wrapper。
    ///
    /// 该 seam 仍走 [`DriveClient`] 的生产请求链，只替换防无限分页的客户端上限。
    pub fn with_pagination_policy(
        client: Arc<crate::drive::client::DriveClient>,
        pagination: PaginationPolicy,
    ) -> Self {
        Self { client, pagination }
    }
}
