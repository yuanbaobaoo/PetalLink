import 'package:petal_link/core/error/app_error.dart';

/// 应用统一结果类型（对标 petal-link-cmp AppResult 密封类）。
///
/// 命令/编排层使用 [AppResult] 回传结果，避免异常穿透。
/// [Ok] 表示成功，包含返回值；[Err] 表示失败，包含 [AppError]。
///
/// 用法：
/// ```dart
/// final result = await someOperation();
/// result.fold(
///   onOk: (value) => print('成功: $value'),
///   onErr: (error) => print('失败: $error'),
/// );
/// ```
sealed class AppResult<T> {
  const AppResult();

  /// 是否成功
  bool get isOk => this is Ok<T>;

  /// 是否失败
  bool get isErr => this is Err<T>;

  /// 解包获取值，失败时抛出 [AppError]
  T unwrap() {
    return switch (this) {
      Ok(:final value) => value,
      Err(:final error) => throw error,
    };
  }

  /// 解包获取值，失败时返回 [defaultValue]
  T unwrapOr(T defaultValue) {
    return switch (this) {
      Ok(:final value) => value,
      Err() => defaultValue,
    };
  }

  /// 映射成功值
  AppResult<R> map<R>(R Function(T value) transform) {
    return switch (this) {
      Ok(:final value) => Ok(transform(value)),
      Err(:final error) => Err(error),
    };
  }

  /// 折叠处理成功与失败分支，统一返回 [R]
  R fold<R>({
    required R Function(T value) onOk,
    required R Function(AppError error) onErr,
  }) {
    return switch (this) {
      Ok(:final value) => onOk(value),
      Err(:final error) => onErr(error),
    };
  }
}

/// 成功结果
final class Ok<T> extends AppResult<T> {
  final T value;

  const Ok(this.value);

  @override
  bool operator ==(Object other) {
    if (identical(this, other)) return true;
    return other is Ok<T> && other.value == value;
  }

  @override
  int get hashCode => value.hashCode;

  @override
  String toString() => 'Ok($value)';
}

/// 失败结果
final class Err<T> extends AppResult<T> {
  final AppError error;

  const Err(this.error);

  @override
  bool operator ==(Object other) {
    if (identical(this, other)) return true;
    return other is Err<T> && other.error == error;
  }

  @override
  int get hashCode => error.hashCode;

  @override
  String toString() => 'Err($error)';
}
