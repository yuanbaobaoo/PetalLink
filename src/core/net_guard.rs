//! 网络与电源状态守卫 —— 断网/睡眠时暂停一切同步操作。
//!
//! 维护全局 Online/Offline 状态。同步引擎各入口通过 is_online() 快速查询；
//! 定时器循环通过 wait_until_online() 阻塞等待网络恢复。
//! 网络判定：每 30s 向华为 API 域名做轻量 TCP connect 探测（443 端口，3s 超时）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::net::TcpStream;
use tokio::time::sleep;

/// 探测目标主机（华为 Drive API 域名）
const PROBE_HOST: &str = "driveapis.cloud.huawei.com.cn:443";
/// 探测间隔（秒）
const PROBE_INTERVAL_SECS: u64 = 30;
/// 单次探测超时
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// wait_until_online 轮询间隔（秒）+ 强制探测后的缩短等待
const POLL_WAIT_SECS: u64 = 2;

/// 全局网络状态：True=在线，false=离线
static ONLINE: AtomicBool = AtomicBool::new(true);

/// 探测任务是否已启动（防止重复 spawn）
static PROBE_STARTED: Mutex<bool> = Mutex::new(false);

/// 探测任务 shutdown 标志（应用退出时置 true，终止探测循环）
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// 查询当前是否在线（零开销，供同步引擎各入口快速判断）。
#[allow(dead_code)]
pub fn is_online() -> bool {
    ONLINE.load(Ordering::SeqCst)
}

/// 启动后台探测任务（幂等，重复调用安全）。
/// 在 tokio 运行时中周期性 TCP 探测目标主机，更新全局 ONLINE 状态。
#[allow(dead_code)]
pub fn start_probe_task() {
    let mut started = PROBE_STARTED.lock();
    if *started {
        return;
    }
    *started = true;
    drop(started);

    tracing::info!("网络探测任务已启动（间隔 {}s）", PROBE_INTERVAL_SECS);
    tokio::spawn(async {
        loop {
            if SHUTDOWN.load(Ordering::SeqCst) {
                tracing::info!("网络探测任务检测到 shutdown，退出循环");
                break;
            }
            let online = probe_once().await;
            let was_online = ONLINE.load(Ordering::SeqCst);
            if online != was_online {
                ONLINE.store(online, Ordering::SeqCst);
                if online {
                    tracing::info!("网络状态：在线（恢复同步）");
                } else {
                    tracing::warn!("网络状态：离线（探测失败，暂停同步）");
                }
            }
            sleep(Duration::from_secs(PROBE_INTERVAL_SECS)).await;
        }
    });
}

/// 通知探测任务退出（应用关闭时调用）。
#[allow(dead_code)]
pub fn shutdown_probe() {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

/// 单次 TCP 探测：connect 到目标主机 443 端口。
#[allow(dead_code)]
async fn probe_once() -> bool {
    match tokio::time::timeout(PROBE_TIMEOUT, TcpStream::connect(PROBE_HOST)).await {
        Ok(Ok(_)) => true,
        Ok(Err(e)) => {
            tracing::debug!(error = %e, "网络探测连接失败");
            false
        }
        Err(_) => {
            tracing::debug!("网络探测超时（{}s）", PROBE_TIMEOUT.as_secs());
            false
        }
    }
}

/// 阻塞等待网络恢复（供定时器循环使用）。
/// 接收 shutdown 闭包，引擎停止时立即返回。
#[allow(dead_code)]
pub async fn wait_until_online<F>(is_shutdown: F)
where
    F: Fn() -> bool,
{
    while !is_online() {
        if is_shutdown() {
            return;
        }
        sleep(Duration::from_secs(POLL_WAIT_SECS)).await;
    }
}

/// 初始化睡眠/唤醒处理（当前为占位 no-op）。
///
/// 当前采用纯探测降级方案（无 NSWorkspace 通知监听）：
/// 睡眠期间 TCP 探测必然超时 → 自动 mark offline；唤醒后探测恢复 → online。
/// 即时性损失最多 30s（一个探测周期），对"睡眠时不该同步"的诉求足够。
/// 因此本函数目前不安装任何监听，仅记录所采用的降级策略。
///
/// 如需即时睡眠感知，可在此处注册 NSWorkspaceWillSleepNotification /
/// NSWorkspaceDidWakeNotification 观察者（需 objc2 observer 回调）。
#[allow(dead_code)]
pub fn init_sleep_handling() {
    tracing::info!(
        "睡眠/唤醒监听：采用纯探测方案（无系统通知，依赖 {}s 周期探测）",
        PROBE_INTERVAL_SECS
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    // 测试串行化锁：用例共享全局 ONLINE 静态状态，
    // cargo test 默认并行运行会相互污染导致 flaky 失败，故强制串行。
    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    #[test]
    fn test_online_default_true() {
        let _guard = TEST_LOCK.lock().unwrap();
        ONLINE.store(true, Ordering::SeqCst);
        assert!(is_online());
    }

    #[test]
    fn test_online_can_flip_to_offline() {
        let _guard = TEST_LOCK.lock().unwrap();
        ONLINE.store(true, Ordering::SeqCst);
        ONLINE.store(false, Ordering::SeqCst);
        assert!(!is_online());
        // 恢复，避免污染其他测试
        ONLINE.store(true, Ordering::SeqCst);
    }
}
