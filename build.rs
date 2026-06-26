// Tauri 构建脚本：
// 1. 读取 tauri.conf.json 生成构建期常量（tauri_build::build）
// 2. 将 assets/ 内的图标同步到 icons/，使 assets/ 成为唯一图源——
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

/// assets/ PNG → icons/ PNG 映射（对齐 tauri.conf.json bundle.icon 所列文件）。
/// 像素尺寸一一对应：assets/icon-NN.png(NN×NN) → icons/NNxNN.png。
const PNG_COPY_MAP: &[(&str, &str)] = &[
    ("icon_32x32.png",       "32x32.png"),
    ("icon_32x32@2x.png",    "32x32@2x.png"),   // 64×64
    ("icon_128x128.png",     "128x128.png"),
    ("icon_128x128@2x.png",  "128x128@2x.png"), // 256×256
    ("icon_512x512.png",     "icon-512.png"),
    ("icon-1024.png",        "icon-1024.png"),
    ("icon-1024.png",        "icon.png"),       // 1024×1024，Tauri 通用入口
];

/// iconutil 标准 .iconset 命名（assets/ 源 → iconset 内文件名）。
/// 覆盖 16~512 及 @2x 全档位，生成多分辨率 icon.icns。
const ICONSET_MAP: &[(&str, &str)] = &[
    ("icon_16x16.png",       "icon_16x16.png"),
    ("icon_16x16@2x.png",    "icon_16x16@2x.png"),
    ("icon_32x32.png",       "icon_32x32.png"),
    ("icon_32x32@2x.png",    "icon_32x32@2x.png"),
    ("icon_128x128.png",     "icon_128x128.png"),
    ("icon_128x128@2x.png",  "icon_128x128@2x.png"),
    ("icon_256x256.png",     "icon_256x256.png"),
    ("icon_256x256@2x.png",  "icon_256x256@2x.png"),
    ("icon_512x512.png",     "icon_512x512.png"),
    ("icon_512x512@2x.png",  "icon_512x512@2x.png"),
];

fn main() {
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
    let out_dir = std::env::var_os("OUT_DIR")
        .ok_or_else(|| std::io::Error::other("OUT_DIR 未设置"))?;
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
        return Err(std::io::Error::other(
            format!("iconutil 退出码 {:?}", status.code()),
        ));
    }
    Ok(())
}
