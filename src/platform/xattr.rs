//! 扩展属性适配 —— 业务层使用稳定逻辑键，平台层映射原生命名空间。
//!
//! macOS 接受 `com.hwcloud.*` 键；Linux 要求普通用户属性位于 `user.*`
//! 命名空间，因此同一逻辑键会持久化为 `user.com.hwcloud.*`。

use std::borrow::Cow;
use std::path::Path;

/// 把业务逻辑键映射为当前平台接受的原生扩展属性键。
fn native_key(key: &str) -> Cow<'_, str> {
    #[cfg(target_os = "linux")]
    {
        const LINUX_NAMESPACES: &[&str] = &["user.", "trusted.", "security.", "system."];
        if LINUX_NAMESPACES
            .iter()
            .any(|namespace| key.starts_with(namespace))
        {
            Cow::Borrowed(key)
        } else {
            Cow::Owned(format!("user.{key}"))
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        Cow::Borrowed(key)
    }
}

/// 读取逻辑键对应的扩展属性；属性不存在时返回 `None`。
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn get(path: &Path, key: &str) -> std::io::Result<Option<Vec<u8>>> {
    xattr::get(path, native_key(key).as_ref())
}

/// 不支持扩展属性的平台返回明确错误，禁止把占位文件误判为普通文件。
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn get(_path: &Path, _key: &str) -> std::io::Result<Option<Vec<u8>>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "当前平台不支持 PetalLink 扩展属性",
    ))
}

/// 写入逻辑键对应的扩展属性。
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn set(path: &Path, key: &str, value: &[u8]) -> std::io::Result<()> {
    xattr::set(path, native_key(key).as_ref(), value)
}

/// 不支持扩展属性的平台拒绝写入，避免创建无法识别的占位文件。
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn set(_path: &Path, _key: &str, _value: &[u8]) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "当前平台不支持 PetalLink 扩展属性",
    ))
}

/// 删除逻辑键对应的扩展属性；属性不存在时由底层按幂等语义处理。
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn remove(path: &Path, key: &str) -> std::io::Result<()> {
    xattr::remove(path, native_key(key).as_ref())
}

/// 不支持扩展属性的平台返回明确错误。
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn remove(_path: &Path, _key: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "当前平台不支持 PetalLink 扩展属性",
    ))
}

#[cfg(test)]
/// 原生键映射与实际读写合同测试。
mod tests {
    use super::{get, native_key, remove, set};

    /// Linux 业务键必须落到 user 命名空间，其他平台保持原键。
    #[test]
    fn logical_key_maps_to_platform_namespace() {
        #[cfg(target_os = "linux")]
        assert_eq!(native_key("com.hwcloud.state"), "user.com.hwcloud.state");
        #[cfg(not(target_os = "linux"))]
        assert_eq!(native_key("com.hwcloud.state"), "com.hwcloud.state");
    }

    /// 已带 Linux 命名空间的键不会被重复添加前缀。
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_namespaced_key_is_not_prefixed_twice() {
        assert_eq!(
            native_key("user.com.hwcloud.state"),
            "user.com.hwcloud.state"
        );
    }

    /// 支持的平台应能通过逻辑键完成扩展属性往返。
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn logical_key_roundtrip_uses_native_xattr() {
        let file = tempfile::NamedTempFile::new().expect("创建扩展属性测试文件失败");
        set(file.path(), "com.hwcloud.state", b"placeholder").expect("写入扩展属性失败");
        assert_eq!(
            get(file.path(), "com.hwcloud.state").expect("读取扩展属性失败"),
            Some(b"placeholder".to_vec())
        );
        remove(file.path(), "com.hwcloud.state").expect("删除扩展属性失败");
        assert_eq!(
            get(file.path(), "com.hwcloud.state").expect("复查扩展属性失败"),
            None
        );
    }
}
