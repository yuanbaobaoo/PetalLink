// Tauri 构建脚本：
// 1. 注入凭证：读取 .env 中的 HWCLOUD_CLIENT_ID / HWCLOUD_CLIENT_SECRET，
//    通过 rustc-env 注入编译期常量（option_env! 可获取）。缺失任一则 panic 阻断构建。
// 2. 读取 tauri.conf.json 生成构建期常量（tauri_build::build）
// 3. 将 assets/ 内的图标同步到 icons/，使 assets/ 成为唯一图源——
//    每次 cargo build / cargo tauri dev 自动覆盖，无需手动复制。
//
// 说明：
// - 托盘图标 assets/menubar-icon.png 由 tray.rs 的 include_bytes! 编译期嵌入，
//   rustc 自动跟踪文件变化，无需此处同步。
// - 应用图标（icons/*.png + icon.icns）被 tauri.conf.json bundle.icon 引用，
//   属于「复制件」，由此处在构建期从 assets/ 重新生成，避免与 assets/ 漂移。

use std::fs;
use std::path::Path;
use std::process::Command;

const ASSETS_DIR: &str = "assets";
const ICONS_DIR: &str = "icons";
const ENV_CLIENT_ID_KEY: &str = "HWCLOUD_CLIENT_ID";
const ENV_CLIENT_SECRET_KEY: &str = "HWCLOUD_CLIENT_SECRET";
const ENV_FILE: &str = ".env";

/// assets/ PNG → icons/ PNG 映射（对齐 tauri.conf.json bundle.icon 所列文件）。
/// 像素尺寸一一对应：assets/icon-NN.png(NN×NN) → icons/NNxNN.png。
const PNG_COPY_MAP: &[(&str, &str)] = &[
    ("icon_32x32.png", "32x32.png"),
    ("icon_32x32@2x.png", "32x32@2x.png"), // 64×64
    ("icon_128x128.png", "128x128.png"),
    ("icon_128x128@2x.png", "128x128@2x.png"), // 256×256
    ("icon_512x512.png", "icon-512.png"),
    ("icon-1024.png", "icon-1024.png"),
    ("icon-1024.png", "icon.png"), // 1024×1024，Tauri 通用入口
];

/// iconutil 标准 .iconset 命名（assets/ 源 → iconset 内文件名）。
/// 覆盖 16~512 及 @2x 全档位，生成多分辨率 icon.icns。
const ICONSET_MAP: &[(&str, &str)] = &[
    ("icon_16x16.png", "icon_16x16.png"),
    ("icon_16x16@2x.png", "icon_16x16@2x.png"),
    ("icon_32x32.png", "icon_32x32.png"),
    ("icon_32x32@2x.png", "icon_32x32@2x.png"),
    ("icon_128x128.png", "icon_128x128.png"),
    ("icon_128x128@2x.png", "icon_128x128@2x.png"),
    ("icon_256x256.png", "icon_256x256.png"),
    ("icon_256x256@2x.png", "icon_256x256@2x.png"),
    ("icon_512x512.png", "icon_512x512.png"),
    ("icon_512x512@2x.png", "icon_512x512@2x.png"),
];

fn main() {
    // ★ 最早阶段：注入凭证（缺失则 panic 阻断构建）
    inject_env_credentials();

    tauri_build::build();

    // assets/ 变化时重新同步图标（tauri.conf.json 由 tauri_build 自行声明跟踪，
    // 此处额外声明以防 tauri_build 未覆盖时配置变更漏跑）
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-changed=tauri.conf.json");

    if let Err(e) = sync_icons() {
        // 同步失败不阻断构建（图标缺失时 Tauri 会回退到默认图标），仅告警
        println!("cargo:warning=图标同步 assets/ → icons/ 失败：{e}");
    }
}

/// 将 .env 中的 HWCLOUD_CLIENT_ID / HWCLOUD_CLIENT_SECRET 注入编译期环境变量。
///
/// 优先级：已显式设置的环境变量 > .env 文件。缺失任一则 panic 阻断构建。
///
/// - `cargo:rustc-env=KEY=VALUE` 使 `option_env!("KEY")` 在编译期可见。
/// - `cargo:rerun-if-changed=.env` 确保凭证变更触发重新编译。
fn inject_env_credentials() {
    // 检测 cargo 构建环境是否已设置（用户显式 export / 命令行前缀）
    let env_id = std::env::var(ENV_CLIENT_ID_KEY)
        .ok()
        .filter(|v| !v.is_empty());
    let env_secret = std::env::var(ENV_CLIENT_SECRET_KEY)
        .ok()
        .filter(|v| !v.is_empty());

    let (client_id, client_secret) = if let (Some(client_id), Some(client_secret)) =
        (env_id, env_secret)
    {
        // 已通过环境变量显式设置，直接使用
        println!("cargo:warning=使用构建环境变量 {ENV_CLIENT_ID_KEY} / {ENV_CLIENT_SECRET_KEY}");
        (client_id, client_secret)
    } else {
        // 从 .env 文件读取
        let env_path = Path::new(ENV_FILE);
        if !env_path.exists() {
            panic!(
                "{ENV_FILE} 不存在！请复制 .env.example 为 .env 并填入 {ENV_CLIENT_ID_KEY} 和 {ENV_CLIENT_SECRET_KEY}。"
            );
        }

        let content = fs::read_to_string(env_path).unwrap_or_else(|e| {
            panic!("读取 {ENV_FILE} 失败：{e}");
        });

        let mut id = String::new();
        let mut secret = String::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                match k.trim() {
                    ENV_CLIENT_ID_KEY => id = v.trim().to_string(),
                    ENV_CLIENT_SECRET_KEY => secret = v.trim().to_string(),
                    _ => {}
                }
            }
        }

        // 校验：两者必须同时非空
        if id.is_empty() && secret.is_empty() {
            panic!(
                "{ENV_FILE} 中 {ENV_CLIENT_ID_KEY} 和 {ENV_CLIENT_SECRET_KEY} 均为空。请填入真实凭据。"
            );
        }
        if id.is_empty() {
            panic!("{ENV_FILE} 中 {ENV_CLIENT_ID_KEY} 为空。请填入真实凭据。");
        }
        if secret.is_empty() {
            panic!("{ENV_FILE} 中 {ENV_CLIENT_SECRET_KEY} 为空。请填入真实凭据。");
        }

        println!("cargo:warning=已从 {ENV_FILE} 注入 {ENV_CLIENT_ID_KEY} / {ENV_CLIENT_SECRET_KEY} 到编译期常量");
        (id, secret)
    };

    // 注入 rustc 编译期环境变量（option_env! 可见）
    println!("cargo:rustc-env={ENV_CLIENT_ID_KEY}={client_id}");
    println!("cargo:rustc-env={ENV_CLIENT_SECRET_KEY}={client_secret}");

    // .env 变化时重新运行 build.rs
    println!("cargo:rerun-if-changed={ENV_FILE}");
    println!("cargo:rerun-if-changed=.env.example");
}

/// 将 assets/ 图标同步到 icons/：复制 PNG + 重新生成 icon.icns。
fn sync_icons() -> std::io::Result<()> {
    fs::create_dir_all(ICONS_DIR)?;

    for (src, dst) in PNG_COPY_MAP {
        let s = Path::new(ASSETS_DIR).join(src);
        let d = Path::new(ICONS_DIR).join(dst);
        if s.exists() {
            fs::copy(&s, &d)?;
        } else {
            println!("cargo:warning=图标源缺失，跳过：{}", s.display());
        }
    }

    #[cfg(target_os = "macos")]
    regenerate_icns()?;

    Ok(())
}

/// 由 assets/ 各档位 PNG 组装 .iconset，再用 iconutil 生成多分辨率 icon.icns。
/// iconset 放在 OUT_DIR，避免污染项目树。
#[cfg(target_os = "macos")]
fn regenerate_icns() -> std::io::Result<()> {
    let out_dir =
        std::env::var_os("OUT_DIR").ok_or_else(|| std::io::Error::other("OUT_DIR 未设置"))?;
    let iconset = Path::new(&out_dir).join("app.iconset");
    let _ = fs::remove_dir_all(&iconset);
    fs::create_dir_all(&iconset)?;

    for (src, name) in ICONSET_MAP {
        let s = Path::new(ASSETS_DIR).join(src);
        if s.exists() {
            fs::copy(&s, iconset.join(name))?;
        }
    }

    let icns_out = Path::new(ICONS_DIR).join("icon.icns");
    let status = Command::new("iconutil")
        .args(["-c", "icns"])
        .arg(&iconset)
        .arg("-o")
        .arg(&icns_out)
        .status()?;

    if !status.success() {
        return Err(std::io::Error::other(format!(
            "iconutil 退出码 {:?}",
            status.code()
        )));
    }
    Ok(())
}
