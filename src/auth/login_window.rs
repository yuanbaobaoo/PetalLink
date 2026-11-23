//! 内嵌 OAuth 登录界面（底部抽屉优先，独立窗口兜底）。
//!
//! 首选形态是「真内嵌」：在主窗口里用一个 child webview 当抽屉内容，从底部滑入，
//! 类似平板上的 modal sheet。child webview 创建失败时退回独立窗口，再失败回退系统浏览器。
//!
//! 登录页用应用自己的 webview 打开华为授权页，而不是跳到系统浏览器：
//! - 回调仍走 loopback（`http://127.0.0.1:<port>/oauth/callback`），后端流程完全不变；
//! - `incognito(true)` 在 macOS 上使用 WKWebView 的非持久化存储，不落地任何华为 Cookie；
//! - 只允许 `https` 与本地回环导航，避免登录窗口被降级或跳去明文站点。
//!
//! 创建失败（或运行环境没有可用 webview）时由调用方回退系统浏览器，因此这里是
//! 「尽力而为」的增强，不能成为登录的唯一通道。

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};

/// 底部抽屉在主窗口内的矩形（整数逻辑像素，原点为窗口内容区左上角）。
///
/// 前端已经按 CSS 像素取整，这里用整数避免浮点边界误差，也让生成的 TS 类型是 `number`。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SheetBounds {
    /// 左边距。
    pub x: i32,
    /// 上边距。
    pub y: i32,
    /// 宽度。
    pub width: i32,
    /// 高度。
    pub height: i32,
    /// 整卡高度（含标题区），用于换算滑入行程。
    ///
    /// 前端整卡从「卡片高度」的下方滑入，原生视图必须走同样长的距离，
    /// 否则动画中登录页会短暂压住标题区。
    pub sheet_height: i32,
    /// 窗口 CSS 视口高度（前端 `window.innerHeight`）。
    ///
    /// 原生子视图以外框顶边为原点定位，而 CSS 视口从内容区开始，两者相差一个标题栏。
    /// 这个偏移量只能由「平台报告的内容区高度 − 真实视口高度」求得：macOS 上平台把
    /// 标题栏算进了内容区（`inner_size` == 外框尺寸），单看平台数值无法区分。
    pub viewport_height: i32,
}

/// 抽屉内容 webview 的标签（与兜底窗口区分）。
pub const SHEET_WEBVIEW_LABEL: &str = "oauth-login-sheet";

/// 抽屉滑入/滑出的动画步数与每步间隔（约 240ms，接近系统 sheet 手感）。
const SHEET_ANIMATION_STEPS: u32 = 15;
/// 每步等待毫秒数。
const SHEET_ANIMATION_STEP_MS: u64 = 16;

/// 是否已注册「窗口缩放 → 主 webview 跟随」监听（只注册一次）。
static MAIN_RESIZE_HOOK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 标题栏偏移的可信上限（逻辑像素）。
///
/// 正常窗口的标题栏不超过几十像素；超过这个值说明平台给出的尺寸与真实视口对不上，
/// 此时宁可不补偿也不能把登录页推出可视区。
const SHEET_MAX_INSET_PX: f64 = 200.0;

/// 抽屉当前位置：用于滑出动画与窗口缩放后的重新定位。
static SHEET_BOUNDS: once_cell::sync::Lazy<std::sync::Mutex<Option<SheetBounds>>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(None));

/// 登录窗口标签（重复登录时用于幂等关闭）。
pub const LOGIN_WINDOW_LABEL: &str = "oauth-login";

/// 登录窗口标题。
const LOGIN_WINDOW_TITLE: &str = "登录华为账号";

/// 登录窗口宽度（华为登录页为窄表单，480 足够显示）。
const LOGIN_WINDOW_WIDTH: f64 = 480.0;

/// 登录窗口高度。
const LOGIN_WINDOW_HEIGHT: f64 = 720.0;

/// 伪造完整 Safari UA。
///
/// WKWebView 默认 UA 缺少 `Version/... Safari/...` 尾巴，部分身份提供方会据此判定
/// 「这是应用内浏览器」并拒绝渲染登录页；改成标准 Safari UA 可显著降低被拦截的概率。
const SAFARI_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15";

/// 判断一次导航是否放行。
///
/// 规则：`https` 全部放行（华为登录可能跨多个子域）；`http` 只允许本地回环回调。
pub(crate) fn is_allowed_navigation(url: &url::Url) -> bool {
    match url.scheme() {
        "https" => true,
        "http" => matches!(
            url.host_str(),
            Some("127.0.0.1") | Some("localhost") | Some("[::1]")
        ),
        _ => false,
    }
}

/// 登录界面实际使用的形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginSurface {
    /// 主窗口内的底部抽屉（首选）。
    Sheet,
    /// 兜底的独立窗口。
    Window,
}

impl LoginSurface {
    /// 对外事件里使用的形态名。
    pub fn as_str(self) -> &'static str {
        match self {
            LoginSurface::Sheet => "sheet",
            LoginSurface::Window => "window",
        }
    }
}

/// 打开内嵌登录界面：优先底部抽屉，失败退回独立窗口。
///
/// `bounds` 为抽屉内容区（已扣掉标题栏）在主窗口内的矩形；`None` 时直接用独立窗口。
/// 返回实际形态；`None` 表示都创建失败，调用方应回退系统浏览器。
pub fn open(
    app: &AppHandle,
    authorize_url: &str,
    bounds: Option<SheetBounds>,
) -> Option<LoginSurface> {
    if let Some(bounds) = bounds {
        if open_sheet(app, authorize_url, bounds) {
            return Some(LoginSurface::Sheet);
        }
        tracing::warn!("底部抽屉创建失败，改用独立登录窗口");
    }
    open_window(app, authorize_url).then_some(LoginSurface::Window)
}

/// 在主窗口里创建抽屉内容 webview 并播放滑入动画。
fn open_sheet(app: &AppHandle, authorize_url: &str, bounds: SheetBounds) -> bool {
    close_sheet(app);
    // add_child 定义在 Window 上：取主窗口的 Window 句柄（与 WebviewWindow 同一底层窗口）。
    let Some(window) = app.get_window("main") else {
        tracing::warn!("主窗口不存在，无法创建底部抽屉");
        return false;
    };
    let parsed = match authorize_url.parse::<url::Url>() {
        Ok(url) => url,
        Err(error) => {
            tracing::warn!(%error, "授权 URL 解析失败，回退系统浏览器");
            return false;
        }
    };
    let builder = tauri::WebviewBuilder::new(SHEET_WEBVIEW_LABEL, WebviewUrl::External(parsed))
        .incognito(true)
        .user_agent(SAFARI_USER_AGENT)
        .on_navigation(is_allowed_navigation);
    // 原生子视图的坐标系从**窗口外框**顶边起算，而前端给的是**内容区**（webview 视口）
    // 坐标；两者相差一个标题栏高度。不补偿这一截，登录页就会整体上移、压住标题区按钮。
    let inset = frame_inset_top(&window, bounds.viewport_height);
    // 起始位置放在窗口下沿之外，再滑到目标位置。
    let target_y = f64::from(bounds.y) + inset;
    // 外框坐标系下的可视区底边：起点必须越过它才算完全滑出。
    let frame_height = f64::from(bounds.viewport_height) + inset;
    // 起点在可视区下方：整卡行程 == 卡片高度，原生视图按同样的位移下探。
    let start_y = (target_y + f64::from(bounds.sheet_height)).max(frame_height);
    let start = LogicalPosition::new(f64::from(bounds.x), start_y);
    let size = LogicalSize::new(f64::from(bounds.width), f64::from(bounds.height));
    if let Err(error) = window.add_child(builder, start, size) {
        tracing::warn!(%error, "创建抽屉 webview 失败");
        return false;
    }
    if let Ok(mut current) = SHEET_BOUNDS.lock() {
        *current = Some(bounds);
    }
    // 打印请求位置与平台回报的实际位置：坐标原点若与 CSS 视口不一致，这里能立刻看出来。
    let actual = app
        .get_webview(SHEET_WEBVIEW_LABEL)
        .and_then(|webview| webview.position().ok());
    tracing::info!(
        css_x = bounds.x,
        css_y = bounds.y,
        css_height = bounds.height,
        inset,
        target_y,
        start_y,
        actual = ?actual,
        frame_height,
        scale = window.scale_factor().ok(),
        "已创建内嵌登录抽屉"
    );
    animate(app.clone(), start_y, target_y);
    ensure_main_resize_hook(app, &window);
    true
}

/// 让主 webview 在抽屉存在时跟随窗口尺寸变化。
///
/// 加了 child webview 之后，主 webview 不再自动跟随窗口缩放（平台侧的布局失效），
/// 结果是登录页按旧视口排版、卡片和按钮被裁到窗口外，用户连关闭按钮都点不到。
/// 这里在窗口尺寸变化时显式把主 webview 设成新尺寸，前端随之重算抽屉矩形。
fn ensure_main_resize_hook(app: &AppHandle, window: &tauri::Window) {
    if MAIN_RESIZE_HOOK.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    window.on_window_event(move |event| {
        let tauri::WindowEvent::Resized(size) = event else {
            return;
        };
        // 没有抽屉时主 webview 本来就会自己跟随，不需要干预。
        if handle.get_webview(SHEET_WEBVIEW_LABEL).is_none() {
            return;
        }
        if let Some(main) = handle.get_webview("main") {
            if let Err(error) = main.set_size(*size) {
                tracing::warn!(%error, "同步主 webview 尺寸失败");
            }
        }
    });
}

/// 更新抽屉位置与尺寸（窗口缩放后调用，不做动画）。
pub fn update_sheet_bounds(app: &AppHandle, bounds: SheetBounds) {
    let Some(webview) = app.get_webview(SHEET_WEBVIEW_LABEL) else {
        return;
    };
    // 与创建时同一条换算：内容区 CSS 坐标 → 外框坐标。
    let inset = app
        .get_window("main")
        .map(|window| frame_inset_top(&window, bounds.viewport_height))
        .unwrap_or_default();
    let _ = webview.set_position(LogicalPosition::new(
        f64::from(bounds.x),
        f64::from(bounds.y) + inset,
    ));
    let _ = webview.set_size(LogicalSize::new(
        f64::from(bounds.width),
        f64::from(bounds.height),
    ));
    if let Ok(mut current) = SHEET_BOUNDS.lock() {
        *current = Some(bounds);
    }
}

/// 关闭抽屉内容 webview（幂等）。
fn close_sheet(app: &AppHandle) {
    if let Ok(mut current) = SHEET_BOUNDS.lock() {
        *current = None;
    }
    if let Some(webview) = app.get_webview(SHEET_WEBVIEW_LABEL) {
        if let Err(error) = webview.close() {
            tracing::warn!(%error, "关闭登录抽屉失败");
        }
    }
}

/// 抽屉纵向位置动画：从 `from_y` 平滑滑到 `to_y`（ease-out）。
fn animate(app: AppHandle, from_y: f64, to_y: f64) {
    tauri::async_runtime::spawn(async move {
        let total = f64::from(SHEET_ANIMATION_STEPS);
        for step in 1..=SHEET_ANIMATION_STEPS {
            let progress = f64::from(step) / total;
            // ease-out cubic：起步快、收尾慢，贴近系统 sheet 手感。
            let eased = 1.0 - (1.0 - progress).powi(3);
            let y = from_y + (to_y - from_y) * eased;
            let Some(webview) = app.get_webview(SHEET_WEBVIEW_LABEL) else {
                // webview 已被关闭（用户取消/登录完成），停止动画。
                return;
            };
            let x = SHEET_BOUNDS
                .lock()
                .ok()
                .and_then(|current| current.map(|bounds| f64::from(bounds.x)))
                .unwrap_or(0.0);
            let _ = webview.set_position(LogicalPosition::new(x, y));
            tokio::time::sleep(std::time::Duration::from_millis(SHEET_ANIMATION_STEP_MS)).await;
        }
    });
}

/// 窗口外框顶边到内容区顶边的距离（逻辑像素，即标题栏高度）。
///
/// 原生子视图以外框顶边为原点定位，而前端给的是 CSS 视口（内容区）坐标，
/// 两者正好差这一截；无边框窗口或全屏时为 0。
fn frame_inset_top(window: &tauri::Window, viewport_height: i32) -> f64 {
    let scale = window.scale_factor().unwrap_or(1.0);
    // 防御 scale 为 0：除零会得到 inf，进而把子视图放到屏幕外。
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let Some(size) = window.inner_size().ok() else {
        return 0.0;
    };
    // 平台报告的逻辑高度（macOS 上含标题栏）减去真实视口高度，差值就是标题栏。
    titlebar_inset(f64::from(size.height) / scale, viewport_height)
}

/// 由「平台报告的逻辑高度」与「真实 CSS 视口高度」求标题栏偏移。
///
/// 只接受 0 到 SHEET_MAX_INSET_PX 之间的值：异常值宁可不补偿，也不能把登录页甩出可视区。
fn titlebar_inset(reported_height: f64, viewport_height: i32) -> f64 {
    let inset = reported_height - f64::from(viewport_height);
    if (0.0..=SHEET_MAX_INSET_PX).contains(&inset) {
        inset
    } else {
        tracing::warn!(
            reported_height,
            viewport_height,
            "标题栏偏移异常，按 0 处理"
        );
        0.0
    }
}

/// 打开独立登录窗口，返回是否成功创建。
///
/// 抽屉不可用时的兜底形态；失败时调用方应回退系统浏览器。
fn open_window(app: &AppHandle, authorize_url: &str) -> bool {
    // 重复点击登录：先关掉上一次残留的窗口，避免出现多个登录窗。
    close(app);
    let parsed = match authorize_url.parse::<url::Url>() {
        Ok(url) => url,
        Err(error) => {
            tracing::warn!(%error, "授权 URL 解析失败，回退系统浏览器");
            return false;
        }
    };
    let builder = WebviewWindowBuilder::new(app, LOGIN_WINDOW_LABEL, WebviewUrl::External(parsed))
        .title(LOGIN_WINDOW_TITLE)
        .inner_size(LOGIN_WINDOW_WIDTH, LOGIN_WINDOW_HEIGHT)
        .resizable(true)
        .center()
        .focused(true)
        // 不持久化 Cookie：每次登录都要输入账号密码，应用内不保留华为会话。
        .incognito(true)
        .user_agent(SAFARI_USER_AGENT)
        // 只允许 https 与本地回环，防止页面把窗口导航到明文站点。
        .on_navigation(is_allowed_navigation);
    match builder.build() {
        Ok(window) => {
            // 用户直接关掉登录窗口 = 取消授权：立刻收掉 loopback 服务，
            // 不必让主界面空等到 5 分钟超时。
            window.on_window_event(|event| {
                if matches!(event, tauri::WindowEvent::Destroyed) {
                    tauri::async_runtime::spawn(async move {
                        crate::runtime::AUTH_SERVICE.cancel_authorize().await;
                    });
                }
            });
            tracing::info!("已打开内嵌登录窗口");
            true
        }
        Err(error) => {
            tracing::warn!(%error, "内嵌登录窗口创建失败，回退系统浏览器");
            false
        }
    }
}

/// 关闭内嵌登录界面（幂等；抽屉与独立窗口都会收掉）。
pub fn close(app: &AppHandle) {
    close_sheet(app);
    if let Some(window) = app.get_webview_window(LOGIN_WINDOW_LABEL) {
        if let Err(error) = window.close() {
            tracing::warn!(%error, "关闭内嵌登录窗口失败");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_allowed_navigation, titlebar_inset};

    /// macOS 把标题栏算进了内容高度：差值就是需要补偿的偏移。
    #[test]
    fn inset_is_reported_height_minus_viewport() {
        assert_eq!(titlebar_inset(800.0, 768), 32.0);
    }

    /// 平台报告的就是真实内容高度时不需要补偿（无边框窗口、全屏）。
    #[test]
    fn inset_is_zero_without_chrome() {
        assert_eq!(titlebar_inset(768.0, 768), 0.0);
    }

    /// 异常差值一律按 0 处理：宁可错位几个像素，也不能把登录页甩出可视区。
    #[test]
    fn bogus_inset_falls_back_to_zero() {
        assert_eq!(titlebar_inset(1200.0, 768), 0.0);
        // 视口比平台报告的还高：负偏移同样不可信。
        assert_eq!(titlebar_inset(700.0, 768), 0.0);
    }
    /// https 一律放行（登录可能跨多个华为子域）。
    #[test]
    fn allows_any_https_navigation() {
        assert!(is_allowed_navigation(
            &"https://oauth-login.cloud.huawei.com/oauth2/v3/authorize?x=1"
                .parse()
                .unwrap()
        ));
        assert!(is_allowed_navigation(
            &"https://id.cloud.huawei.com/whatever".parse().unwrap()
        ));
    }

    /// 本地回环回调必须放行，否则拿不到 code。
    #[test]
    fn allows_loopback_callback() {
        assert!(is_allowed_navigation(
            &"http://127.0.0.1:57321/oauth/callback?code=abc"
                .parse()
                .unwrap()
        ));
        assert!(is_allowed_navigation(
            &"http://localhost:57321/oauth/callback".parse().unwrap()
        ));
    }

    /// 其他明文站点与非 http 协议一律拦截。
    #[test]
    fn blocks_plain_http_and_exotic_schemes() {
        assert!(!is_allowed_navigation(
            &"http://example.com/login".parse().unwrap()
        ));
        assert!(!is_allowed_navigation(
            &"file:///etc/passwd".parse().unwrap()
        ));
        assert!(!is_allowed_navigation(
            &"tauri://localhost/index.html".parse().unwrap()
        ));
    }
}
