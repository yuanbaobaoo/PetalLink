//! 强类型的持久传输状态、操作与错误分类。

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 持久传输任务的生命周期状态。
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransferState {
    Pending = 0,
    Running = 1,
    WaitingForNetwork = 2,
    BackingOff = 3,
    VerifyingRemote = 4,
    RestartRequired = 5,
    Completed = 6,
    Failed = 7,
    Canceled = 8,
}

impl TransferState {
    /// 终态：任务已结束，不再占用任何资源。
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TransferState::Completed | TransferState::Failed | TransferState::Canceled
        )
    }

    /// 该状态下的任务是否占用其路径：进行中、退避中、等待核验远端结果的任务
    /// 都可能仍在改写两端数据，路径争议处理与动作过滤以此为据。
    /// RestartRequired 仅在已知远端结果（remote_result_file_id 非空）时占用路径——
    /// 没有远端结果的滞留记录可由用户清除或直接重试，不阻塞路径。
    /// 注意与数据安全场景的区分：释放本地空间等删除类判定应使用更保守的
    /// `!is_terminal()`，滞留的 RestartRequired 也不能放行。
    pub fn occupies_path(self, remote_result_file_id: Option<&str>) -> bool {
        matches!(
            self,
            TransferState::Pending
                | TransferState::Running
                | TransferState::WaitingForNetwork
                | TransferState::BackingOff
                | TransferState::VerifyingRemote
        ) || (self == TransferState::RestartRequired
            && remote_result_file_id.is_some_and(|file_id| !file_id.trim().is_empty()))
    }
}

/// 持久传输任务代表的文件操作。
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransferOperation {
    Create = 0,
    Update = 1,
    Download = 2,
    DownloadUpdate = 3,
    Delete = 4,
    Move = 5,
    Rename = 6,
    CreateFolder = 7,
}

/// 可持久的结构化传输失败类型。
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransferErrorKind {
    Network = 0,
    Timeout = 1,
    Auth = 2,
    RateLimit = 3,
    Server = 4,
    Quota = 5,
    Permission = 6,
    Validation = 7,
    SessionExpired = 8,
    RemoteAmbiguous = 9,
    LocalChanged = 10,
    Unknown = 11,
    /// 目标目录存在同名但内容不一致的远端文件，等待用户选择处理方式。
    NameConflict = 12,
}

/// 乐观传输状态迁移被拒绝的原因。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransitionError {
    #[error("illegal transfer transition from {from:?} to {to:?}")]
    IllegalTransition {
        from: TransferState,
        to: TransferState,
    },
    #[error("stale transfer revision for task {task_id}: expected {expected_revision}")]
    StaleRevision {
        task_id: i64,
        expected_revision: i64,
    },
    #[error("transfer task {task_id} was not found")]
    NotFound { task_id: i64 },
    #[error("invalid stored value {value} for {field}")]
    InvalidStoredValue { field: &'static str, value: i32 },
    #[error("transfer database operation failed: {message}")]
    Database { message: String },
}

impl From<rusqlite::Error> for TransitionError {
    /// 将 SQLite 错误包装为传输迁移错误。
    fn from(error: rusqlite::Error) -> Self {
        Self::Database {
            message: error.to_string(),
        }
    }
}

/// 为持久枚举生成与整数之间的严格转换。
macro_rules! impl_persistent_enum {
    ($enum:ty, $field:literal, [$($value:literal => $variant:path),+ $(,)?]) => {
        impl From<$enum> for i32 {
            fn from(value: $enum) -> Self {
                value as i32
            }
        }

        impl TryFrom<i32> for $enum {
            type Error = TransitionError;

            /// 严格将数据库整数值解析为枚举。
            fn try_from(value: i32) -> Result<Self, Self::Error> {
                match value {
                    $($value => Ok($variant),)+
                    _ => Err(TransitionError::InvalidStoredValue {
                        field: $field,
                        value,
                    }),
                }
            }
        }
    };
}

impl_persistent_enum!(
    TransferState,
    "state",
    [
        0 => TransferState::Pending,
        1 => TransferState::Running,
        2 => TransferState::WaitingForNetwork,
        3 => TransferState::BackingOff,
        4 => TransferState::VerifyingRemote,
        5 => TransferState::RestartRequired,
        6 => TransferState::Completed,
        7 => TransferState::Failed,
        8 => TransferState::Canceled,
    ]
);
impl_persistent_enum!(
    TransferOperation,
    "operation",
    [
        0 => TransferOperation::Create,
        1 => TransferOperation::Update,
        2 => TransferOperation::Download,
        3 => TransferOperation::DownloadUpdate,
        4 => TransferOperation::Delete,
        5 => TransferOperation::Move,
        6 => TransferOperation::Rename,
        7 => TransferOperation::CreateFolder,
    ]
);
impl_persistent_enum!(
    TransferErrorKind,
    "error_kind",
    [
        0 => TransferErrorKind::Network,
        1 => TransferErrorKind::Timeout,
        2 => TransferErrorKind::Auth,
        3 => TransferErrorKind::RateLimit,
        4 => TransferErrorKind::Server,
        5 => TransferErrorKind::Quota,
        6 => TransferErrorKind::Permission,
        7 => TransferErrorKind::Validation,
        8 => TransferErrorKind::SessionExpired,
        9 => TransferErrorKind::RemoteAmbiguous,
        10 => TransferErrorKind::LocalChanged,
        11 => TransferErrorKind::Unknown,
        12 => TransferErrorKind::NameConflict,
    ]
);

/// 判断 `from -> to` 是否为声明过的传输生命周期边。
pub const fn can_transition(from: TransferState, to: TransferState) -> bool {
    use TransferState::*;

    matches!(
        (from, to),
        (
            Pending,
            Running | WaitingForNetwork | RestartRequired | Failed | Canceled
        ) | (
            Running,
            WaitingForNetwork
                | BackingOff
                | VerifyingRemote
                | RestartRequired
                | Completed
                | Failed
                | Canceled
        ) | (
            WaitingForNetwork,
            Running | RestartRequired | Failed | Canceled
        ) | (BackingOff, Running | RestartRequired | Failed | Canceled)
            | (
                VerifyingRemote,
                Running
                    | WaitingForNetwork
                    | BackingOff
                    | RestartRequired
                    | Completed
                    | Failed
                    | Canceled
            )
            | (
                RestartRequired,
                Pending | VerifyingRemote | Failed | Canceled
            )
            | (Failed, Pending | RestartRequired | Canceled)
    )
}
