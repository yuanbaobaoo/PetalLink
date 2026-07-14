//! Drive 客户端响应解码错误语义测试。

use petal_link_lib::drive::client::response_decode_error;
use petal_link_lib::error::{AppError, DriveTransportKind, RequestSemantics};

/// 验证写响应解码失败保留已提交不确定性。
#[test]
fn response_decode_error_preserves_write_submission_uncertainty() {
    let write = response_decode_error("createFolder", RequestSemantics::Write, true, "missing id");
    let read = response_decode_error("list", RequestSemantics::Read, false, "invalid json");

    assert_eq!(write.to_string(), "云端响应异常");
    assert!(matches!(
        write,
        AppError::DriveApi {
            transport_kind: Some(DriveTransportKind::Decode),
            request_may_have_reached_server: true,
            auth_already_replayed: true,
            ..
        }
    ));
    assert!(matches!(
        read,
        AppError::DriveApi {
            transport_kind: Some(DriveTransportKind::Decode),
            request_may_have_reached_server: false,
            auth_already_replayed: false,
            ..
        }
    ));
}
