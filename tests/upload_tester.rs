//! 华为云盘真实文件上传的手工集成测试。
//!
//! 默认忽略；显式运行前需通过 `HWCLOUD_TEST_FILE` 指定本地文件。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use petal_link_lib::auth::service::AuthService;
use petal_link_lib::drive::client::DriveClient;
use petal_link_lib::drive::upload_api::UploadApi;

/// 从环境变量或 OAuth 授权流程获取测试 token。
async fn get_token() -> Result<String> {
    // 优先使用显式提供的测试 token。
    if let Ok(token) = std::env::var("HWCLOUD_TEST_TOKEN") {
        if !token.is_empty() {
            eprintln!("✓ 从环境变量读取 token");
            return Ok(token);
        }
    }

    // 未找到有效 token 时启动 OAuth 授权流程。
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
            eprintln!(
                "  token 已保存到 ~/Library/Application Support/io.github.yuanbaobaoo.PetalLink/token.bin"
            );
            Ok(token_pair.access_token)
        }
        Err(error) => {
            eprintln!("✗ OAuth 授权失败: {error}");
            eprintln!("  请通过环境变量提供 token:");
            eprintln!("    export HWCLOUD_TEST_TOKEN=\"<access_token>\"");
            eprintln!("    HWCLOUD_TEST_FILE=\"<file_path>\" cargo test --test upload_tester -- --ignored --nocapture");
            Err(anyhow!("OAuth 授权失败: {error}"))
        }
    }
}

/// 使用真实 token 将指定文件上传到华为云盘。
#[tokio::test]
#[ignore = "需要真实华为云盘令牌与 HWCLOUD_TEST_FILE"]
async fn upload_real_file_to_huawei_cloud() -> Result<()> {
    // 加载 OAuth 客户端配置。
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter("info,petal_link_lib=info")
        .try_init()
        .ok();

    let file_path = std::env::var("HWCLOUD_TEST_FILE")
        .context("缺少 HWCLOUD_TEST_FILE；请将其设置为待上传文件路径后使用 --ignored 显式运行")?;
    let file_path = PathBuf::from(file_path);
    if !file_path.exists() {
        return Err(anyhow!("文件不存在: {}", file_path.display()));
    }

    let file_size = file_path
        .metadata()
        .with_context(|| format!("无法读取文件元数据: {}", file_path.display()))?
        .len();
    eprintln!(
        "文件: {} ({:.1} MB)",
        file_path.display(),
        file_size as f64 / 1_048_576.0
    );

    let token = get_token().await?;

    let auth = Arc::new(AuthService::new());
    let client = Arc::new(DriveClient::new(auth));
    let api = UploadApi::new(client);

    eprintln!("开始上传（分片续传，预期 308 → 308 → 200）...");
    let start = Instant::now();

    match api.upload_resume_with_token(&file_path, None, &token).await {
        Ok(file) => {
            let elapsed = start.elapsed();
            eprintln!("═══════════════════════════════════════");
            eprintln!("✅ 上传成功！");
            eprintln!("   fileId:    {}", file.id);
            eprintln!("   fileName:  {}", file.name);
            eprintln!(
                "   size:      {} bytes ({:.1} MB)",
                file.size,
                file.size as f64 / 1_048_576.0
            );
            eprintln!("   耗时:      {:.1}s", elapsed.as_secs_f64());
            eprintln!("═══════════════════════════════════════");
            Ok(())
        }
        Err(error) => {
            let elapsed = start.elapsed();
            eprintln!("═══════════════════════════════════════");
            eprintln!("❌ 上传失败 (耗时 {:.1}s)", elapsed.as_secs_f64());
            eprintln!("   错误: {error}");
            eprintln!("═══════════════════════════════════════");
            Err(anyhow!("上传失败: {error}"))
        }
    }
}
