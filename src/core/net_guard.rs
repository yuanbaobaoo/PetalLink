//! 网络与电源状态守卫 —— 断网/睡眠时暂停一切同步操作。
//!
//! 维护全局 Online/Offline 状态。同步引擎各入口通过 is_online() 快速查询；
//! 定时器循环通过 wait_until_online() 阻塞等待网络恢复。
//! 网络判定：每 30s 向华为 API 域名做轻量 TCP connect 探测（443 端口，3s 超时）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use tokio::net::TcpStream;
use tokio::sync::broadcast;
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

/// 稳定网络状态发生的真实转换。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkTransition {
    Online,
    Offline,
}

#[derive(Debug)]
pub(crate) struct NetworkStateMachine {
    online: bool,
    consecutive_successes: u8,
}

impl NetworkStateMachine {
    pub(crate) fn new(online: bool) -> Self {
        Self {
            online,
            consecutive_successes: 0,
        }
    }

    pub(crate) fn is_online(&self) -> bool {
        self.online
    }

    pub(crate) fn observe(&mut self, probe_succeeded: bool) -> Option<NetworkTransition> {
        if !probe_succeeded {
            self.consecutive_successes = 0;
            if self.online {
                self.online = false;
                return Some(NetworkTransition::Offline);
            }
            return None;
        }

        if self.online {
            self.consecutive_successes = 0;
            return None;
        }

        self.consecutive_successes = self.consecutive_successes.saturating_add(1);
        if self.consecutive_successes < 2 {
            return None;
        }
        self.online = true;
        self.consecutive_successes = 0;
        Some(NetworkTransition::Online)
    }
}

#[derive(Debug, Default)]
struct ProbeLifecycle {
    generation: u64,
    running: bool,
}

impl ProbeLifecycle {
    fn start(&mut self) -> Option<u64> {
        if self.running {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.generation = 1;
        }
        self.running = true;
        Some(self.generation)
    }

    fn shutdown(&mut self) -> bool {
        if !self.running {
            return false;
        }
        self.running = false;
        true
    }

    fn accepts(&self, generation: u64) -> bool {
        self.running && self.generation == generation
    }

    fn finish(&mut self, generation: u64) -> bool {
        if !self.accepts(generation) {
            return false;
        }
        self.running = false;
        true
    }
}

#[derive(Debug)]
struct ProbeRuntime {
    lifecycle: ProbeLifecycle,
    network: NetworkStateMachine,
}

impl Default for ProbeRuntime {
    fn default() -> Self {
        Self {
            lifecycle: ProbeLifecycle::default(),
            network: NetworkStateMachine::new(true),
        }
    }
}

static PROBE_RUNTIME: Lazy<Mutex<ProbeRuntime>> = Lazy::new(|| Mutex::new(ProbeRuntime::default()));
static TRANSITIONS: Lazy<broadcast::Sender<NetworkTransition>> = Lazy::new(|| {
    let (sender, _) = broadcast::channel(16);
    sender
});

struct ProbeGenerationGuard {
    generation: u64,
}

impl Drop for ProbeGenerationGuard {
    fn drop(&mut self) {
        finish_probe_generation(self.generation);
    }
}

/// 查询当前是否在线（零开销，供同步引擎各入口快速判断）。
#[allow(dead_code)]
pub fn is_online() -> bool {
    ONLINE.load(Ordering::SeqCst)
}

/// 订阅稳定网络转换；只会收到真实 Offline/Online 状态变化。
pub fn subscribe() -> broadcast::Receiver<NetworkTransition> {
    TRANSITIONS.subscribe()
}

/// Feed a real request-layer transport failure into the same stable level machine used by TCP
/// probes. This produces at most one Online→Offline edge; recovery still requires two successful
/// probes, preventing a WaitingForNetwork transition from immediately retrying in a hot loop.
pub fn report_request_network_failure() -> bool {
    let mut runtime = PROBE_RUNTIME.lock();
    publish_request_network_failure(&mut runtime.network, &ONLINE, &TRANSITIONS)
}

fn publish_request_network_failure(
    network: &mut NetworkStateMachine,
    online_mirror: &AtomicBool,
    transitions: &broadcast::Sender<NetworkTransition>,
) -> bool {
    let Some(transition) = network.observe(false) else {
        return false;
    };
    online_mirror.store(false, Ordering::SeqCst);
    let _ = transitions.send(transition);
    true
}

/// 启动后台探测任务（幂等，重复调用安全）。
/// 在 tokio 运行时中周期性 TCP 探测目标主机，更新全局 ONLINE 状态。
#[allow(dead_code)]
pub fn start_probe_task() {
    let generation = {
        let mut runtime = PROBE_RUNTIME.lock();
        let Some(generation) = runtime.lifecycle.start() else {
            return;
        };
        runtime.network = NetworkStateMachine::new(ONLINE.load(Ordering::SeqCst));
        generation
    };

    tracing::info!("网络探测任务已启动（间隔 {}s）", PROBE_INTERVAL_SECS);
    tokio::spawn(async move {
        let _generation_guard = ProbeGenerationGuard { generation };
        loop {
            if !generation_is_active(generation) {
                tracing::info!("网络探测任务检测到 shutdown，退出循环");
                break;
            }
            let probe_succeeded = probe_once().await;
            if !record_probe_result(generation, probe_succeeded) {
                break;
            }
            sleep(Duration::from_secs(PROBE_INTERVAL_SECS)).await;
        }
    });
}

/// 通知探测任务退出（应用关闭时调用）。
#[allow(dead_code)]
pub fn shutdown_probe() {
    let stopped = PROBE_RUNTIME.lock().lifecycle.shutdown();
    if stopped {
        tracing::info!("网络探测任务已请求停止");
    }
}

fn generation_is_active(generation: u64) -> bool {
    PROBE_RUNTIME.lock().lifecycle.accepts(generation)
}

fn finish_probe_generation(generation: u64) -> bool {
    PROBE_RUNTIME.lock().lifecycle.finish(generation)
}

fn record_probe_result(generation: u64, probe_succeeded: bool) -> bool {
    let mut runtime = PROBE_RUNTIME.lock();
    let was_online = runtime.network.is_online();
    if !publish_probe_result(
        &mut runtime,
        generation,
        probe_succeeded,
        &ONLINE,
        &TRANSITIONS,
    ) {
        return false;
    }
    let online = runtime.network.is_online();
    if online != was_online {
        if online {
            tracing::info!("网络状态：在线（恢复同步）");
        } else {
            tracing::warn!("网络状态：离线（探测失败，暂停同步）");
        }
    }
    true
}

fn publish_probe_result(
    runtime: &mut ProbeRuntime,
    generation: u64,
    probe_succeeded: bool,
    online_mirror: &AtomicBool,
    transitions: &broadcast::Sender<NetworkTransition>,
) -> bool {
    if !runtime.lifecycle.accepts(generation) {
        return false;
    }
    let Some(transition) = runtime.network.observe(probe_succeeded) else {
        return true;
    };
    online_mirror.store(runtime.network.is_online(), Ordering::SeqCst);
    let _ = transitions.send(transition);
    true
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

    #[test]
    fn offline_recovery_requires_two_consecutive_successes() {
        let mut state = NetworkStateMachine::new(true);

        assert_eq!(state.observe(false), Some(NetworkTransition::Offline));
        assert!(!state.is_online());
        assert_eq!(state.observe(true), None);
        assert!(!state.is_online());
        assert_eq!(state.observe(false), None);
        assert_eq!(state.observe(true), None);
        assert_eq!(state.observe(true), Some(NetworkTransition::Online));
        assert!(state.is_online());
        assert_eq!(state.observe(true), None);
    }

    #[test]
    fn flapping_emits_exactly_one_event_per_stable_transition() {
        let mut state = NetworkStateMachine::new(true);
        let transitions: Vec<_> = [false, true, true, false, false, true, true]
            .into_iter()
            .filter_map(|success| state.observe(success))
            .collect();

        assert_eq!(
            transitions,
            vec![
                NetworkTransition::Offline,
                NetworkTransition::Online,
                NetworkTransition::Offline,
                NetworkTransition::Online,
            ]
        );
    }

    #[test]
    fn probe_lifecycle_is_idempotent_restart_safe_and_rejects_old_generation() {
        let mut lifecycle = ProbeLifecycle::default();
        assert!(!lifecycle.shutdown());

        let first = lifecycle.start().expect("first start creates generation");
        assert_eq!(lifecycle.start(), None);
        assert!(lifecycle.accepts(first));

        assert!(lifecycle.shutdown());
        assert!(!lifecycle.shutdown());
        assert!(!lifecycle.accepts(first));

        let second = lifecycle.start().expect("restart creates generation");
        assert_ne!(first, second);
        assert!(!lifecycle.accepts(first));
        assert!(lifecycle.accepts(second));

        assert!(!lifecycle.finish(first));
        assert!(lifecycle.accepts(second));
        assert!(lifecycle.finish(second));
        assert!(!lifecycle.accepts(second));
    }

    #[test]
    fn broadcast_publishes_only_real_stable_transitions() {
        let mut runtime = ProbeRuntime::default();
        let generation = runtime.lifecycle.start().unwrap();
        let (sender, mut receiver) = broadcast::channel(8);
        let online = AtomicBool::new(true);

        assert!(publish_probe_result(
            &mut runtime,
            generation,
            false,
            &online,
            &sender,
        ));
        assert_eq!(receiver.try_recv(), Ok(NetworkTransition::Offline));
        assert!(!online.load(Ordering::SeqCst));

        assert!(publish_probe_result(
            &mut runtime,
            generation,
            false,
            &online,
            &sender,
        ));
        assert!(publish_probe_result(
            &mut runtime,
            generation,
            true,
            &online,
            &sender,
        ));
        assert!(matches!(
            receiver.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        assert!(!online.load(Ordering::SeqCst));

        assert!(publish_probe_result(
            &mut runtime,
            generation,
            true,
            &online,
            &sender,
        ));
        assert_eq!(receiver.try_recv(), Ok(NetworkTransition::Online));
        assert!(online.load(Ordering::SeqCst));

        assert!(matches!(
            receiver.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn request_layer_failure_reports_offline_once_then_uses_stable_recovery() {
        let mut network = NetworkStateMachine::new(true);
        let online = AtomicBool::new(true);
        let (sender, mut receiver) = broadcast::channel(4);

        assert!(publish_request_network_failure(
            &mut network,
            &online,
            &sender
        ));
        assert_eq!(receiver.try_recv(), Ok(NetworkTransition::Offline));
        assert!(!publish_request_network_failure(
            &mut network,
            &online,
            &sender
        ));
        assert!(matches!(
            receiver.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        assert_eq!(network.observe(true), None);
        assert_eq!(network.observe(true), Some(NetworkTransition::Online));
    }
}
