//! 命令行探查工具 —— 华为 Drive /drive/v1/changes 接口行为。
//!
//! 用法:
//!   cargo run --bin changes_probe
//!
//! 自动获取 token：先尝试 HWCLOUD_TEST_TOKEN 环境变量，
//! 若无则启动 OAuth 授权流程（打开浏览器）。
//! 注：与 upload_tester 一样，无法读取主程序加密的 token.bin。

use petal_link_lib::auth::service::AuthService;

// 对齐主程序 constants::DRIVE_API_BASE 的 host（driveapis 复数）
const DRIVE_API_BASE: &str = "https://driveapis.cloud.huawei.com.cn";

async fn get_token() -> String {
    if let Ok(t) = std::env::var("HWCLOUD_TEST_TOKEN") {
        if !t.is_empty() {
            eprintln!("✓ 从环境变量读取 token");
            return t;
        }
    }
    eprintln!("未找到 HWCLOUD_TEST_TOKEN，启动 OAuth 授权...");
    let auth = AuthService::new();
    match auth.authorize(9999).await {
        Ok(token_pair) => token_pair.access_token,
        Err(e) => {
            eprintln!("✗ OAuth 授权失败: {e}");
            eprintln!("  请: export HWCLOUD_TEST_TOKEN=\"<access_token>\"");
            std::process::exit(1);
        }
    }
}

/// 打印一次请求的完整信息：URL、状态码、响应体。
async fn probe(client: &reqwest::Client, token: &str, label: &str, url: &str) {
    eprintln!("\n═══════════════════════════════════════════════════════════");
    eprintln!("探测: {label}");
    eprintln!("URL : {url}");
    let resp = match client.get(url).bearer_auth(token).send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("✗ 请求失败: {e}");
            return;
        }
    };
    let status = resp.status();
    eprintln!("状态: {status}");
    let body = resp.text().await.unwrap_or_else(|e| format!("<读取响应体失败: {e}>"));
    // 尝试 pretty-print JSON
    match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(v) => eprintln!("响应:\n{}", serde_json::to_string_pretty(&v).unwrap_or(body)),
        Err(_) => eprintln!("响应(非JSON):\n{body}"),
    }
    eprintln!("═══════════════════════════════════════════════════════════");
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter("info,petal_link_lib=info")
        .init();

    let token = get_token().await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();

    // 1) 不带 cursor，首次拉取（看是否返回初始 cursor / 或报错要 cursor）
    probe(&client, &token, "1. 首次无 cursor", &format!("{DRIVE_API_BASE}/drive/v1/changes")).await;

    // 2) 带 fields=*（华为 /about 接口要求 fields=*，看 changes 是否同理）
    probe(&client, &token, "2. fields=* 无 cursor", &format!("{DRIVE_API_BASE}/drive/v1/changes?fields=*")).await;

    // 3.1) GDrive 协议: getStartPageToken 取初始游标
    probe(&client, &token, "3. getStartPageToken", &format!("{DRIVE_API_BASE}/drive/v1/changes/startPageToken")).await;

    // 3.2) 带 pageSize 限制（看分页字段名）
    probe(&client, &token, "4. pageSize=1", &format!("{DRIVE_API_BASE}/drive/v1/changes?fields=*&pageSize=1")).await;

    // 4) 带 cursor 重试（先用空字符串看报错信息，了解 cursor 参数名）
    probe(&client, &token, "5. cursor=空", &format!("{DRIVE_API_BASE}/drive/v1/changes?fields=*&cursor=")).await;

    eprintln!("\n✓ 探查完成。请把以上响应贴入设计文档第 6 节，用于阶段三字段映射。");
}
