//! 持久化传输状态公开合同测试。

use petal_link_lib::sync::transfer_state::{
    can_transition, TransferErrorKind, TransferOperation, TransferState,
};

/// 验证持久化传输状态数值保持稳定。
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

/// 验证持久化传输操作数值保持稳定。
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

/// 验证持久化错误类型数值保持稳定。
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

/// 验证状态机只允许声明过的迁移边。
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
        (Pending, WaitingForNetwork),
        (Pending, RestartRequired),
        (Pending, Failed),
        (Pending, Canceled),
        (Running, WaitingForNetwork),
        (Running, BackingOff),
        (Running, VerifyingRemote),
        (Running, RestartRequired),
        (Running, Completed),
        (Running, Failed),
        (Running, Canceled),
        (WaitingForNetwork, Running),
        (WaitingForNetwork, RestartRequired),
        (WaitingForNetwork, Failed),
        (WaitingForNetwork, Canceled),
        (BackingOff, Running),
        (BackingOff, RestartRequired),
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
        (RestartRequired, VerifyingRemote),
        (RestartRequired, Failed),
        (RestartRequired, Canceled),
        (Failed, Pending),
        (Failed, RestartRequired),
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

/// 验证完成与取消状态均为终态。
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
