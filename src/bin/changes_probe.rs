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
    // 优先复用已存的 token.bin（避免每次探查都重新 OAuth 授权）
    let auth = AuthService::new();
    if let Ok(t) = auth.ensure_valid_access_token().await {
        eprintln!("✓ 复用已存 token");
        return t;
    }
    eprintln!("未找到有效 token，启动 OAuth 授权...");
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

    // 已知：getStartCursor 端点 = /drive/v1/changes/getStartCursor，返回 {startCursor:"..."}
    // 本次重点：用真实 startCursor 调 /changes，确认变更响应结构（数组名/变更字段/removed 标志）

    // 1) 取 startCursor
    let start_cursor: String = {
        let resp = client.get(format!("{DRIVE_API_BASE}/drive/v1/changes/getStartCursor"))
            .bearer_auth(&token).send().await.unwrap();
        let v: serde_json::Value = resp.json().await.unwrap();
        eprintln!("✓ startCursor = {}", v["startCursor"]);
        v["startCursor"].as_str().unwrap().to_string()
    };

    // 2) 用历史小 cursor（如 "1"）调 /changes，看真实变更记录的字段结构
    //    startCursor=当前点，从更早的 cursor 能拿到历史变更记录
    probe(&client, &token, "2. /changes with cursor=1（看历史变更结构）", &format!("{DRIVE_API_BASE}/drive/v1/changes?fields=*&pageSize=3&cursor=1")).await;

    // 3) 顺带确认 newStartCursor 与当前 startCursor 的关系
    probe(&client, &token, "3. /changes with current startCursor", &format!("{DRIVE_API_BASE}/drive/v1/changes?fields=*&pageSize=1&cursor={start_cursor}")).await;

    eprintln!("\n✓ 探查完成。请把 #2 的响应贴给我，确认单条变更的字段结构（removed 标志 / file 字段）。");
}
