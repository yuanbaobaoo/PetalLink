//! Typed persistent transfer state, operation, and error classifications.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Persistent transfer task lifecycle state.
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

/// Operation represented by a persistent transfer task.
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

/// Structured transfer failure classification.
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
}

/// A rejected optimistic transfer transition.
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
    fn from(error: rusqlite::Error) -> Self {
        Self::Database {
            message: error.to_string(),
        }
    }
}

macro_rules! impl_persistent_enum {
    ($enum:ty, $field:literal, [$($value:literal => $variant:path),+ $(,)?]) => {
        impl From<$enum> for i32 {
            fn from(value: $enum) -> Self {
                value as i32
            }
        }

        impl TryFrom<i32> for $enum {
            type Error = TransitionError;

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
    ]
);

/// Returns whether `from -> to` is a declared transfer lifecycle edge.
pub const fn can_transition(from: TransferState, to: TransferState) -> bool {
    use TransferState::*;

    matches!(
        (from, to),
        (Pending, Running | Canceled)
            | (
                Running,
                WaitingForNetwork
                    | BackingOff
                    | VerifyingRemote
                    | RestartRequired
                    | Completed
                    | Failed
                    | Canceled
            )
            | (WaitingForNetwork, Running | Failed | Canceled)
            | (BackingOff, Running | Failed | Canceled)
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
            | (RestartRequired, Pending | Failed | Canceled)
            | (Failed, Pending | Canceled)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_state_values_are_stable() {
        let values = [
            (TransferState::Pending, 0),
            (TransferState::Running, 1),
            (TransferState::WaitingForNetwork, 2),
            (TransferState::BackingOff, 3),
            (TransferState::VerifyingRemote, 4),
            (TransferState::RestartRequired, 5),
            (TransferState::Completed, 6),
            (TransferState::Failed, 7),
            (TransferState::Canceled, 8),
        ];

        for (state, value) in values {
            assert_eq!(i32::from(state), value);
            assert_eq!(TransferState::try_from(value).unwrap(), state);
        }
        assert!(TransferState::try_from(9).is_err());
    }

    #[test]
    fn persisted_operation_values_are_stable() {
        let values = [
            (TransferOperation::Create, 0),
            (TransferOperation::Update, 1),
            (TransferOperation::Download, 2),
            (TransferOperation::DownloadUpdate, 3),
            (TransferOperation::Delete, 4),
            (TransferOperation::Move, 5),
            (TransferOperation::Rename, 6),
            (TransferOperation::CreateFolder, 7),
        ];

        for (operation, value) in values {
            assert_eq!(i32::from(operation), value);
            assert_eq!(TransferOperation::try_from(value).unwrap(), operation);
        }
        assert!(TransferOperation::try_from(8).is_err());
    }

    #[test]
    fn persisted_error_kind_values_are_stable() {
        let values = [
            (TransferErrorKind::Network, 0),
            (TransferErrorKind::Timeout, 1),
            (TransferErrorKind::Auth, 2),
            (TransferErrorKind::RateLimit, 3),
            (TransferErrorKind::Server, 4),
            (TransferErrorKind::Quota, 5),
            (TransferErrorKind::Permission, 6),
            (TransferErrorKind::Validation, 7),
            (TransferErrorKind::SessionExpired, 8),
            (TransferErrorKind::RemoteAmbiguous, 9),
            (TransferErrorKind::LocalChanged, 10),
            (TransferErrorKind::Unknown, 11),
        ];

        for (kind, value) in values {
            assert_eq!(i32::from(kind), value);
            assert_eq!(TransferErrorKind::try_from(value).unwrap(), kind);
        }
        assert!(TransferErrorKind::try_from(12).is_err());
    }

    #[test]
    fn transition_matrix_allows_only_declared_edges() {
        use TransferState::*;

        let states = [
            Pending,
            Running,
            WaitingForNetwork,
            BackingOff,
            VerifyingRemote,
            RestartRequired,
            Completed,
            Failed,
            Canceled,
        ];
        let allowed = [
            (Pending, Running),
            (Pending, Canceled),
            (Running, WaitingForNetwork),
            (Running, BackingOff),
            (Running, VerifyingRemote),
            (Running, RestartRequired),
            (Running, Completed),
            (Running, Failed),
            (Running, Canceled),
            (WaitingForNetwork, Running),
            (WaitingForNetwork, Failed),
            (WaitingForNetwork, Canceled),
            (BackingOff, Running),
            (BackingOff, Failed),
            (BackingOff, Canceled),
            (VerifyingRemote, Running),
            (VerifyingRemote, WaitingForNetwork),
            (VerifyingRemote, BackingOff),
            (VerifyingRemote, RestartRequired),
            (VerifyingRemote, Completed),
            (VerifyingRemote, Failed),
            (VerifyingRemote, Canceled),
            (RestartRequired, Pending),
            (RestartRequired, Failed),
            (RestartRequired, Canceled),
            (Failed, Pending),
            (Failed, Canceled),
        ];

        for from in states {
            for to in states {
                assert_eq!(
                    can_transition(from, to),
                    allowed.contains(&(from, to)),
                    "unexpected transition result for {from:?} -> {to:?}"
                );
            }
        }
    }

    #[test]
    fn completed_and_canceled_are_terminal() {
        use TransferState::*;

        let states = [
            Pending,
            Running,
            WaitingForNetwork,
            BackingOff,
            VerifyingRemote,
            RestartRequired,
            Completed,
            Failed,
            Canceled,
        ];
        for to in states {
            assert!(!can_transition(Completed, to));
            assert!(!can_transition(Canceled, to));
        }
    }
}
