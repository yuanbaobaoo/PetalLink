import 'package:dio/dio.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/error/app_error.dart';

void main() {
  group('AppError 子类结构', () {
    test('AuthError 携带子码与 kind', () {
      const error = AuthError(
        authCode: AuthErrorCode.denied,
        message: '授权被拒绝',
      );

      expect(error.kind, 'Auth');
      expect(error.code, 'denied');
      expect(error.message, '授权被拒绝');
      expect(error.statusCode, isNull);
      expect(error.errorCode, isNull);
      expect(error, isA<AppError>());
    });

    test('TokenError 携带子码与 kind', () {
      const error = TokenError(
        tokenCode: TokenErrorCode.refreshFailed,
        message: 'Token 刷新失败，请重新登录',
      );

      expect(error.kind, 'Token');
      expect(error.code, 'refresh_failed');
    });

    test('DriveApiError 携带完整结构化元数据', () {
      const error = DriveApiError(
        driveCode: DriveApiErrorCode.fromStatus,
        message: '云端请求失败 (404)',
        statusCode: 404,
        errorCode: 'fileNotFound',
        requestMayHaveReachedServer: true,
        authAlreadyReplayed: true,
      );

      expect(error.kind, 'DriveApi');
      expect(error.code, 'from_status');
      expect(error.statusCode, 404);
      expect(error.errorCode, 'fileNotFound');
      expect(error.driveStatus, 404);
      expect(error.requestMayHaveReachedServer, isTrue);
      expect(error.authAlreadyReplayed, isTrue);
    });

    test('ConfigError / GenericError 只有 message', () {
      const config = ConfigError(message: '配置缺失');
      const generic = GenericError(message: '文件操作失败');

      expect(config.kind, 'Config');
      expect(config.code, isNull);
      expect(generic.kind, 'Generic');
      expect(generic.driveStatus, isNull);
    });

    test('QuotaExceededError 携带 required/remaining', () {
      const error = QuotaExceededError(
        required: 100,
        remaining: 10,
        message: '空间不足：需要 100 字节，剩余 10 字节',
      );

      expect(error.kind, 'QuotaExceeded');
      expect(error.required, 100);
      expect(error.remaining, 10);
    });
  });

  group('AppError 工厂（对齐 Rust error.rs）', () {
    test('authCancelled', () {
      final error = AppError.authCancelled();
      expect(error, isA<AuthError>());
      expect(error.code, 'cancelled');
      expect(error.message, '用户取消授权');
    });

    test('authStateMismatch / authTimeout / authBrowserLaunchFailed', () {
      expect(AppError.authStateMismatch().code, 'state_mismatch');
      expect(AppError.authTimeout().code, 'timeout');
      expect(AppError.authBrowserLaunchFailed().code, 'browser_launch_failed');
    });

    test('authDenied 带描述与不带描述', () {
      expect(AppError.authDenied('access_denied').message, '授权失败：access_denied');
      expect(AppError.authDenied(null).message, '授权被拒绝');
    });

    test('authInvalidCode / authTokenResponseInvalid', () {
      expect(AppError.authInvalidCode().code, 'invalid_code');
      expect(AppError.authTokenResponseInvalid().code, 'token_response_invalid');
    });

    test('tokenNotLoggedIn / tokenRefreshFailed', () {
      expect(AppError.tokenNotLoggedIn().code, 'not_logged_in');
      expect(AppError.tokenRefreshFailed().message, 'Token 刷新失败，请重新登录');
      expect(AppError.tokenRefreshFailed('网络错误').message, 'Token 刷新失败：网络错误');
    });

    test('driveFromStatus 解析华为错误码', () {
      final error = AppError.driveFromStatus(403, '{"errorCode": 1101}');
      expect(error, isA<DriveApiError>());
      expect(error.statusCode, 403);
      expect(error.errorCode, '1101');
      expect(error.message, '云端请求失败 (403)');
      // 读语义：未到达服务端
      expect((error as DriveApiError).requestMayHaveReachedServer, isFalse);
    });

    test('driveFromResponse 写语义标记可能已提交', () {
      final error = AppError.driveFromResponse(
        500,
        '{}',
        semantics: RequestSemantics.write,
        authAlreadyReplayed: true,
      ) as DriveApiError;

      expect(error.requestMayHaveReachedServer, isTrue);
      expect(error.authAlreadyReplayed, isTrue);
    });

    test('driveUploadSessionExpired 始终要求远端复核', () {
      final error = AppError.driveUploadSessionExpired(404) as DriveApiError;
      expect(error.errorCode, 'upload_session_expired');
      expect(error.requestMayHaveReachedServer, isTrue);
    });

    test('driveQuotaExceeded', () {
      final error = AppError.driveQuotaExceeded();
      expect(error.code, 'quota_exceeded');
      expect(error.errorCode, 'quota_exceeded');
      expect(error.message, '云盘空间不足');
    });

    test('driveTransport：写请求 connect 阶段失败可安全重试', () {
      final error = AppError.driveTransport(
        DriveTransportKind.connect,
        semantics: RequestSemantics.write,
      ) as DriveApiError;
      // connect 阶段失败：请求未发出，不会到达服务端
      expect(error.requestMayHaveReachedServer, isFalse);
      expect(error.code, 'network');
    });

    test('driveTransport：写请求 timeout 阶段失败可能已提交', () {
      final error = AppError.driveTransport(
        DriveTransportKind.timeout,
        semantics: RequestSemantics.write,
      ) as DriveApiError;
      expect(error.requestMayHaveReachedServer, isTrue);
      expect(error.message, '网络连接失败，请检查网络');
    });

    test('driveTransport：读请求失败不标记提交', () {
      final error = AppError.driveTransport(
        DriveTransportKind.timeout,
      ) as DriveApiError;
      expect(error.requestMayHaveReachedServer, isFalse);
    });

    test('driveTransport：decode 失败消息为云端响应异常', () {
      final error = AppError.driveTransport(DriveTransportKind.decode);
      expect(error.message, '云端响应异常');
    });

    test('quotaExceeded 工厂拼接字节数', () {
      final error = AppError.quotaExceeded(1024, 512) as QuotaExceededError;
      expect(error.required, 1024);
      expect(error.remaining, 512);
      expect(error.message, '空间不足：需要 1024 字节，剩余 512 字节');
    });

    test('config / generic 工厂', () {
      expect(AppError.config('路径不安全'), isA<ConfigError>());
      expect(AppError.generic('数据解析失败'), isA<GenericError>());
    });
  });

  group('AppError.toJson（对齐 Rust 自定义 Serialize 扁平结构）', () {
    test('Auth 序列化为五字段扁平结构', () {
      final json = AppError.authCancelled().toJson();
      expect(json, {
        'kind': 'Auth',
        'code': 'cancelled',
        'message': '用户取消授权',
        'status_code': null,
        'error_code': null,
      });
    });

    test('DriveApi 序列化携带 status_code 与 error_code', () {
      final json = AppError.driveFromStatus(404, '{"errorCode":"notFound"}').toJson();
      expect(json['kind'], 'DriveApi');
      expect(json['code'], 'from_status');
      expect(json['status_code'], 404);
      expect(json['error_code'], 'notFound');
      expect(json['message'], isA<String>());
    });

    test('Config / Generic 序列化 code 为 null', () {
      expect(AppError.config('x').toJson()['code'], isNull);
      expect(AppError.generic('x').toJson()['kind'], 'Generic');
    });
  });

  group('AppError.parseHuaweiErrorCode', () {
    test('顶层 errorCode 字符串', () {
      expect(AppError.parseHuaweiErrorCode('{"errorCode":"abc"}'), 'abc');
    });

    test('顶层 errorCode 数字转字符串', () {
      expect(AppError.parseHuaweiErrorCode('{"errorCode":1101}'), '1101');
    });

    test('嵌套 error.errorCode', () {
      expect(
        AppError.parseHuaweiErrorCode('{"error":{"errorCode":"nested"}}'),
        'nested',
      );
    });

    test('非 JSON / 无错误码返回 null', () {
      expect(AppError.parseHuaweiErrorCode('not json'), isNull);
      expect(AppError.parseHuaweiErrorCode('{"foo":1}'), isNull);
      expect(AppError.parseHuaweiErrorCode(''), isNull);
    });
  });

  group('RetryAfter', () {
    test('解析 delta-seconds', () {
      final retry = RetryAfter.tryParse('30');
      expect(retry, isA<RetryAfterDelay>());
      expect((retry as RetryAfterDelay).seconds, 30);
      expect(retry.nextRetryAt(1000), 31000);
    });

    test('解析 IMF-fixdate', () {
      final retry = RetryAfter.tryParse('Wed, 21 Oct 2015 07:28:00 GMT');
      expect(retry, isA<RetryAfterAt>());
      // 2015-10-21 07:28:00 GMT = 1445412480000
      expect(retry!.nextRetryAt(0), 1445412480000);
    });

    test('AtUnixMs 不早于当前时刻', () {
      const retry = RetryAfterAt(1000);
      expect(retry.nextRetryAt(5000), 5000);
      expect(retry.nextRetryAt(500), 1000);
    });

    test('非法输入返回 null', () {
      expect(RetryAfter.tryParse(null), isNull);
      expect(RetryAfter.tryParse('garbage'), isNull);
      expect(RetryAfter.tryParse(''), isNull);
    });
  });

  group('AppError.fromDioException', () {
    DioException dioErr(
      DioExceptionType type, {
      String? message,
      Response<dynamic>? response,
      Object? error,
    }) {
      return DioException(
        requestOptions: RequestOptions(path: ''),
        type: type,
        message: message,
        response: response,
        error: error,
      );
    }

    test('connectionError 分类为 connect', () {
      final error = AppError.fromDioException(
        dioErr(DioExceptionType.connectionError, message: '无网络'),
      ) as DriveApiError;

      expect(error.transportKind, DriveTransportKind.connect);
      expect(error.code, 'network');
    });

    test('超时类型分类为 timeout', () {
      for (final type in [
        DioExceptionType.connectionTimeout,
        DioExceptionType.sendTimeout,
        DioExceptionType.receiveTimeout,
      ]) {
        final error = AppError.fromDioException(dioErr(type)) as DriveApiError;
        expect(error.transportKind, DriveTransportKind.timeout);
      }
    });

    test('cancel 归为 GenericError', () {
      final error = AppError.fromDioException(
        dioErr(DioExceptionType.cancel, message: '已取消'),
      );
      expect(error, isA<GenericError>());
      expect(error.message, '已取消');
    });

    test('带响应时按服务端错误处理并解析 Retry-After', () {
      final response = Response(
        requestOptions: RequestOptions(path: ''),
        statusCode: 429,
        data: '{"errorCode":"rateLimited"}',
        headers: Headers()
          ..set('Retry-After', '60'),
      );
      final error = AppError.fromDioException(
        dioErr(DioExceptionType.badResponse, response: response),
        semantics: RequestSemantics.write,
      ) as DriveApiError;

      expect(error.statusCode, 429);
      expect(error.errorCode, 'rateLimited');
      expect(error.retryAfter, isA<RetryAfterDelay>());
      expect(error.requestMayHaveReachedServer, isTrue);
    });

    test('写请求 connect 失败不标记提交，timeout 失败标记提交', () {
      final connect = AppError.fromDioException(
        dioErr(DioExceptionType.connectionError),
        semantics: RequestSemantics.write,
      ) as DriveApiError;
      expect(connect.requestMayHaveReachedServer, isFalse);

      final timeout = AppError.fromDioException(
        dioErr(DioExceptionType.sendTimeout),
        semantics: RequestSemantics.write,
      ) as DriveApiError;
      expect(timeout.requestMayHaveReachedServer, isTrue);
    });

    test('内嵌 AppError 直接透传（401 刷新失败）', () {
      final inner = AppError.tokenRefreshFailed();
      final error = AppError.fromDioException(
        dioErr(DioExceptionType.unknown, error: inner),
      );
      expect(error, same(inner));
    });

    test('authAlreadyReplayed 透传', () {
      final error = AppError.fromDioException(
        dioErr(DioExceptionType.connectionError),
        authAlreadyReplayed: true,
      ) as DriveApiError;
      expect(error.authAlreadyReplayed, isTrue);
    });
  });

  group('AppError.fromStatusCode', () {
    test('任意状态码归为 DriveApi fromStatus', () {
      final error = AppError.fromStatusCode(500, '{"errorCode":"internal"}');
      expect(error, isA<DriveApiError>());
      expect(error.statusCode, 500);
      expect(error.errorCode, 'internal');
      expect(error.message, '云端请求失败 (500)');
    });
  });

  group('AppError.toString', () {
    test('包含 kind、message 与非空元数据', () {
      final str = AppError.driveFromStatus(500, '{"errorCode":"x"}').toString();
      expect(str, contains('DriveApi'));
      expect(str, contains('云端请求失败 (500)'));
      expect(str, contains('statusCode: 500'));
      expect(str, contains('errorCode: x'));
    });

    test('省略空元数据', () {
      final str = const GenericError(message: '简单错误').toString();
      expect(str, contains('Generic'));
      expect(str, contains('简单错误'));
      expect(str.contains('statusCode'), isFalse);
    });
  });
}
