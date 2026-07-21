/// Drive API 端点常量（严格对齐 Rust 原版 `src/constants.rs`）。
library;

/// Drive REST API base（CRUD / 搜索 / 缩略图 / 下载 / 变更 / 配额）
const String driveApiBase =
    'https://driveapis.cloud.huawei.com.cn/drive/v1';

/// Upload API base（multipart 小文件 + resume 断点续传）
const String uploadApiBase =
    'https://driveapis.cloud.huawei.com.cn/upload/drive/v1';
