//! 命令行上传测试工具 —— >20MB 文件上传到华为云盘。
//!
//! 用法:
//!   cargo run --bin upload-tester -- <file_path>
//!
//! 自动获取 token：先尝试从环境变量/Keychain/文件读取，
//! 如果都没有，则启动 OAuth 授权流程（打开浏览器，用户点击授权即可）。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use petal_link_lib::auth::service::AuthService;
use petal_link_lib::drive::client::DriveClient;
use petal_link_lib::drive::upload_api::UploadApi;

fn try_read_token_file(path: &std::path::Path) -> Option<String> {
    if !path.exists() { return None; }
    let raw = std::fs::read_to_string(path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let token = parsed.get("access_token").and_then(|v| v.as_str())?;
    // 跳过测试 mock token
    if token.starts_with("at-") && token.len() < 20 { return None; }
    Some(token.to_string())
}

async fn get_token() -> String {
    // 1. 环境变量
    if let Ok(t) = std::env::var("HWCLOUD_TEST_TOKEN") {
        if !t.is_empty() {
            eprintln!("✓ 从环境变量读取 token");
            return t;
        }
    }

    // 2. PetalLink Keychain
    if let Ok(entry) = keyring::Entry::new(
        petal_link_lib::constants::BUNDLE_IDENTIFIER,
        "hwcloud.access_token",
    ) {
        if let Ok(t) = entry.get_password() {
            eprintln!("✓ 从 Keychain 读取 token");
            return t;
        }
    }

    // 3. PetalLink Application Support token.json（FileTokenStore 降级路径）
    if let Some(dir) = dirs::data_dir() {
        let path = dir.join("io.github.yuanbaobaoo.PetalLink").join("token.json");
        if let Some(t) = try_read_token_file(&path) {
            eprintln!("✓ 从 {} 读取 token", path.display());
            return t;
        }
    }

    // 4. 未找到有效 token → 启动 OAuth 授权流程（自动获取新 token）
    eprintln!();
    eprintln!("══════════════════════════════════════════════");
    eprintln!("  未找到有效 token，开始 OAuth 授权流程...");
    eprintln!("  浏览器将打开华为授权页面，请点击 [授权] 按钮。");
    eprintln!("══════════════════════════════════════════════");
    eprintln!();

    let auth = AuthService::new();
    match auth.authorize(9999).await {
        Ok(token_pair) => {
            eprintln!();
            eprintln!("✓ OAuth 授权成功！");
            eprintln!("  token 已保存到 ~/Library/Application Support/io.github.yuanbaobaoo.PetalLink/token.json");
            token_pair.access_token
        }
        Err(e) => {
            eprintln!("✗ OAuth 授权失败: {e}");
            eprintln!("  请通过环境变量提供 token:");
            eprintln!("    export HWCLOUD_TEST_TOKEN=\"<access_token>\"");
            eprintln!("    cargo run --bin upload-tester -- <file_path>");
            std::process::exit(1);
        }
    }
}

#[tokio::main]
async fn main() {
    // 加载 .env（client_id / client_secret）
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter("info,petal_link_lib=info")
        .init();

    let file_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| {
            eprintln!("用法: cargo run --bin upload-tester -- <file_path>");
            std::process::exit(1);
        });
    let file_path = PathBuf::from(&file_path);
    if !file_path.exists() {
        eprintln!("错误: 文件不存在: {}", file_path.display());
        std::process::exit(1);
    }

    let file_size = file_path.metadata().unwrap().len();
    eprintln!("文件: {} ({:.1} MB)", file_path.display(), file_size as f64 / 1_048_576.0);

    let token = get_token().await;

    let auth = Arc::new(petal_link_lib::auth::service::AuthService::new());
    let client = Arc::new(DriveClient::new(auth));
    let api = UploadApi::new(client);

    eprintln!("开始上传（分片续传，预期 308 → 308 → 200）...");
    let start = Instant::now();

    match api.upload_resume_with_token(&file_path, None, &token).await {
        Ok(f) => {
            let elapsed = start.elapsed();
            eprintln!("═══════════════════════════════════════");
            eprintln!("✅ 上传成功！");
            eprintln!("   fileId:    {}", f.id);
            eprintln!("   fileName:  {}", f.name);
            eprintln!("   size:      {} bytes ({:.1} MB)", f.size, f.size as f64 / 1_048_576.0);
            eprintln!("   耗时:      {:.1}s", elapsed.as_secs_f64());
            eprintln!("═══════════════════════════════════════");
        }
        Err(e) => {
            let elapsed = start.elapsed();
            eprintln!("═══════════════════════════════════════");
            eprintln!("❌ 上传失败 (耗时 {:.1}s)", elapsed.as_secs_f64());
            eprintln!("   错误: {}", e);
            eprintln!("═══════════════════════════════════════");
            std::process::exit(1);
        }
    }
}
