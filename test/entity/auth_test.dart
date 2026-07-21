import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/entity/auth.dart';

void main() {
  group('TokenPair', () {
    // ── isExpired / willExpireWithin ────────────────────────────────────

    group('isExpired / willExpireWithin', () {
      test('isExpired returns true when expiresAt is in the past', () {
        final past = DateTime.now()
            .subtract(const Duration(hours: 1))
            .millisecondsSinceEpoch;
        final token = TokenPair(
          accessToken: 'token',
          refreshToken: 'refresh',
          expiresAt: past,
        );

        expect(token.isExpired, isTrue);
      });

      test('isExpired returns false when expiresAt is far in the future', () {
        final future = DateTime.now()
            .add(const Duration(hours: 24))
            .millisecondsSinceEpoch;
        final token = TokenPair(
          accessToken: 'token',
          refreshToken: 'refresh',
          expiresAt: future,
        );

        expect(token.isExpired, isFalse);
      });

      test('willExpireWithin 默认 60 秒缓冲窗口', () {
        final soon = DateTime.now()
            .add(const Duration(seconds: 30))
            .millisecondsSinceEpoch;
        final token = TokenPair(
          accessToken: 'token',
          refreshToken: 'refresh',
          expiresAt: soon,
        );

        // 30 秒后过期：未过期但在 60 秒窗口内
        expect(token.isExpired, isFalse);
        expect(token.willExpireWithin(), isTrue);
      });

      test('willExpireWithin 窗口外返回 false', () {
        final nearFuture = DateTime.now()
            .add(const Duration(seconds: 61))
            .millisecondsSinceEpoch;
        final token = TokenPair(
          accessToken: 'token',
          refreshToken: 'refresh',
          expiresAt: nearFuture,
        );

        expect(token.willExpireWithin(), isFalse);
      });

      test('willExpireWithin 支持自定义缓冲', () {
        final future = DateTime.now()
            .add(const Duration(minutes: 5))
            .millisecondsSinceEpoch;
        final token = TokenPair(
          accessToken: 'token',
          refreshToken: 'refresh',
          expiresAt: future,
        );

        expect(token.willExpireWithin(const Duration(minutes: 10)), isTrue);
        expect(token.willExpireWithin(const Duration(minutes: 1)), isFalse);
      });
    });

    // ── fromTokenResponse ───────────────────────────────────────────────

    group('fromTokenResponse', () {
      test('parses snake_case token endpoint response', () {
        final before = DateTime.now().millisecondsSinceEpoch;
        final token = TokenPair.fromTokenResponse({
          'access_token': 'access-123',
          'refresh_token': 'refresh-456',
          'expires_in': 3600,
          'token_type': 'Bearer',
          'scope': 'https://www.huawei.com/auth/drive/file',
        });
        final after = DateTime.now().millisecondsSinceEpoch;

        expect(token, isNotNull);
        expect(token!.accessToken, 'access-123');
        expect(token.refreshToken, 'refresh-456');
        expect(token.tokenType, 'Bearer');
        expect(token.scope, 'https://www.huawei.com/auth/drive/file');
        expect(token.expiresAt,
            greaterThanOrEqualTo(before + 3600 * 1000));
        expect(token.expiresAt,
            lessThanOrEqualTo(after + 3600 * 1000));
      });

      test('tolerates expires_in as string', () {
        final token = TokenPair.fromTokenResponse({
          'access_token': 'access',
          'expires_in': '7200',
        });

        expect(token, isNotNull);
        expect(token!.isExpired, isFalse);
        expect(
            token.willExpireWithin(const Duration(seconds: 7199)), isFalse);
      });

      test('defaults expires_in to 3600 and token_type to Bearer', () {
        final token = TokenPair.fromTokenResponse({
          'access_token': 'access',
        });

        expect(token, isNotNull);
        expect(token!.refreshToken, '');
        expect(token.tokenType, 'Bearer');
        expect(token.scope, isNull);
        // 默认 1 小时后过期：3 小时窗口内在期内，59 分钟窗口外
        expect(token.willExpireWithin(const Duration(hours: 3)), isTrue);
        expect(token.willExpireWithin(const Duration(minutes: 59)), isFalse);
      });

      test('returns null when access_token missing or empty', () {
        expect(TokenPair.fromTokenResponse({'expires_in': 3600}), isNull);
        expect(
            TokenPair.fromTokenResponse({'access_token': ''}), isNull);
        expect(TokenPair.fromTokenResponse({'access_token': 123}), isNull);
      });
    });

    // ── fromJson / toJson roundtrip ─────────────────────────────────────

    group('fromJson / toJson', () {
      test('roundtrip preserves all fields（snake_case 键）', () {
        const token = TokenPair(
          accessToken: 'my-access-token-123',
          refreshToken: 'my-refresh-token-456',
          expiresAt: 1767225599000,
          tokenType: 'Bearer',
          scope: 'drive',
        );

        final json = token.toJson();
        expect(json['access_token'], 'my-access-token-123');
        expect(json['refresh_token'], 'my-refresh-token-456');
        expect(json['expires_at'], 1767225599000);
        expect(json['token_type'], 'Bearer');
        expect(json['scope'], 'drive');

        final restored = TokenPair.fromJson(json);
        expect(restored.accessToken, token.accessToken);
        expect(restored.refreshToken, token.refreshToken);
        expect(restored.expiresAt, token.expiresAt);
        expect(restored.tokenType, token.tokenType);
        expect(restored.scope, token.scope);
      });

      test('fromJson tolerates expires_at as string', () {
        final token = TokenPair.fromJson({
          'access_token': 'tok',
          'refresh_token': 'ref',
          'expires_at': '1767225599000',
        });

        expect(token.expiresAt, 1767225599000);
      });

      test('fromJson defaults tokenType to Bearer and scope to null', () {
        final token = TokenPair.fromJson({
          'access_token': 'tok',
          'refresh_token': 'ref',
          'expires_at': 1,
        });

        expect(token.tokenType, 'Bearer');
        expect(token.scope, isNull);
      });
    });

    // ── copyWith ────────────────────────────────────────────────────────

    group('copyWith', () {
      test('replaces specified fields', () {
        const token = TokenPair(
          accessToken: 'old-access',
          refreshToken: 'old-refresh',
          expiresAt: 1000,
          tokenType: 'Bearer',
          scope: 'drive',
        );

        final copy = token.copyWith(
          accessToken: 'new-access',
          expiresAt: 2000,
        );

        expect(copy.accessToken, 'new-access');
        expect(copy.refreshToken, 'old-refresh');
        expect(copy.expiresAt, 2000);
        expect(copy.tokenType, 'Bearer');
        expect(copy.scope, 'drive');
      });

      test('explicit null clears scope', () {
        const token = TokenPair(
          accessToken: 'a',
          refreshToken: 'r',
          expiresAt: 1,
          scope: 'drive',
        );

        final cleared = token.copyWith(scope: null);
        expect(cleared.scope, isNull);
        // 不传则保持原值
        expect(token.copyWith().scope, 'drive');
      });
    });
  });

  group('UserInfo', () {
    // ── fromJson 多端点字段别名 ─────────────────────────────────────────

    group('fromJson', () {
      test('picks primary keys directly', () {
        final info = UserInfo.fromJson({
          'sub': 'sub-123',
          'openID': 'openid-456',
          'unionID': 'unionid-789',
          'displayName': '张三',
          'name': 'Zhang San',
          'nickname': '三哥',
          'email': 'zhangsan@example.com',
          'mobile': '13800138000',
          'headPictureURL': 'https://example.com/avatar.png',
        });

        expect(info.sub, 'sub-123');
        expect(info.openId, 'openid-456');
        expect(info.unionId, 'unionid-789');
        expect(info.displayName, '张三');
        expect(info.name, 'Zhang San');
        expect(info.nickname, '三哥');
        expect(info.email, 'zhangsan@example.com');
        expect(info.mobile, '13800138000');
        expect(info.avatarUrl, 'https://example.com/avatar.png');
        expect(info.isAnonymized, isFalse);
      });

      test('picks alias keys（兼容多端点命名）', () {
        final info = UserInfo.fromJson({
          'user_id': 'uid-1',
          'open_id': 'oid-1',
          'unionId': 'unid-1',
          'display_name': 'Alias Name',
          'nick_name': 'Nick',
          'phone_number': '13900139000',
          'avatar_url': 'https://example.com/a.png',
        });

        expect(info.sub, 'uid-1');
        expect(info.openId, 'oid-1');
        expect(info.unionId, 'unid-1');
        expect(info.displayName, 'Alias Name');
        expect(info.nickname, 'Nick');
        expect(info.mobile, '13900139000');
        expect(info.avatarUrl, 'https://example.com/a.png');
      });

      test('skips blank strings and trims whitespace', () {
        final info = UserInfo.fromJson({
          'sub': '   ',
          'userId': '  uid-trimmed  ',
        });

        expect(info.sub, 'uid-trimmed');
      });

      test('displayNameFlag=1 marks anonymized（容忍 String 数字）', () {
        expect(
          UserInfo.fromJson({'displayNameFlag': 1}).isAnonymized,
          isTrue,
        );
        expect(
          UserInfo.fromJson({'displayNameFlag': '1'}).isAnonymized,
          isTrue,
        );
        expect(
          UserInfo.fromJson({'displayNameFlag': 0}).isAnonymized,
          isFalse,
        );
        expect(const UserInfo().isAnonymized, isFalse);
      });
    });

    // ── 展示标签 ────────────────────────────────────────────────────────

    group('primaryLabel / secondaryLabel / initial', () {
      test('primaryLabel 优先 displayName', () {
        const info = UserInfo(
          displayName: '张三',
          mobile: '13800138000',
          name: 'Zhang',
        );

        expect(info.primaryLabel, '张三');
      });

      test('primaryLabel 无 displayName 时走手机号', () {
        const info = UserInfo(
          mobile: '13800138000',
          name: 'Zhang',
        );

        expect(info.primaryLabel, '13800138000');
      });

      test('primaryLabel 依次回退 name/nickname/openId/sub', () {
        expect(const UserInfo(name: 'n', nickname: 'nn').primaryLabel, 'n');
        expect(const UserInfo(nickname: 'nn', openId: 'o').primaryLabel,
            'nn');
        expect(const UserInfo(openId: 'o', sub: 's').primaryLabel, 'o');
        expect(const UserInfo(sub: 's').primaryLabel, 's');
        expect(const UserInfo().primaryLabel, isNull);
      });

      test('secondaryLabel 优先邮箱（与主标不重复）', () {
        const info = UserInfo(
          displayName: '张三',
          email: 'a@b.com',
          mobile: '13800138000',
        );

        expect(info.secondaryLabel, 'a@b.com');
      });

      test('secondaryLabel 邮箱与主标重复时走手机号', () {
        const info = UserInfo(
          displayName: 'a@b.com',
          email: 'a@b.com',
          mobile: '13800138000',
        );

        expect(info.secondaryLabel, '13800138000');
      });

      test('secondaryLabel 匿名账号兜底', () {
        const info = UserInfo(isAnonymized: true);

        expect(info.secondaryLabel, '匿名账号');
      });

      test('secondaryLabel 全部缺失时为 null', () {
        expect(const UserInfo().secondaryLabel, isNull);
      });

      test('initial 取主标首字符（CJK 安全）', () {
        const info = UserInfo(displayName: '张三');

        expect(info.initial, '张');
        expect(const UserInfo().initial, isNull);
      });
    });

    // ── resolveAnonymousAsMobile ────────────────────────────────────────

    group('resolveAnonymousAsMobile', () {
      test('匿名 + 有手机号 → 清掉匿名名走手机号', () {
        const info = UserInfo(
          displayName: '匿名用户',
          mobile: '13800138000',
          isAnonymized: true,
        );

        final resolved = info.resolveAnonymousAsMobile();

        expect(resolved.displayName, isNull);
        expect(resolved.primaryLabel, '13800138000');
      });

      test('非匿名 → 原样返回', () {
        const info = UserInfo(displayName: '张三', mobile: '13800138000');

        final resolved = info.resolveAnonymousAsMobile();

        expect(resolved.displayName, '张三');
      });

      test('匿名但无手机号 → 原样返回', () {
        const info = UserInfo(displayName: '匿名用户', isAnonymized: true);

        final resolved = info.resolveAnonymousAsMobile();

        expect(resolved.displayName, '匿名用户');
      });
    });

    // ── toJson / copyWith ───────────────────────────────────────────────

    group('toJson / copyWith', () {
      test('toJson uses snake_case keys and roundtrips', () {
        const info = UserInfo(
          sub: 's',
          openId: 'o',
          unionId: 'u',
          displayName: 'd',
          name: 'n',
          nickname: 'nn',
          email: 'e',
          mobile: 'm',
          avatarUrl: 'a',
          isAnonymized: true,
        );

        final json = info.toJson();
        expect(json['open_id'], 'o');
        expect(json['union_id'], 'u');
        expect(json['display_name'], 'd');
        expect(json['avatar_url'], 'a');
        expect(json['is_anonymized'], true);

        final restored = UserInfo.fromJson(json);
        expect(restored.sub, 's');
        expect(restored.openId, 'o');
        expect(restored.unionId, 'u');
        expect(restored.displayName, 'd');
        expect(restored.mobile, 'm');
        expect(restored.avatarUrl, 'a');
      });

      test('copyWith replaces fields and explicit null clears', () {
        const info = UserInfo(displayName: 'old', email: 'e@x.com');

        final copy = info.copyWith(displayName: 'new');
        expect(copy.displayName, 'new');
        expect(copy.email, 'e@x.com');

        final cleared = info.copyWith(email: null);
        expect(cleared.email, isNull);
        expect(cleared.displayName, 'old');
      });
    });
  });

  group('AuthState', () {
    test('defaults: 未登录 / 未配置密钥 / 端口 9999', () {
      const state = AuthState();

      expect(state.loggedIn, isFalse);
      expect(state.secretConfigured, isFalse);
      expect(state.callbackPort, 9999);
    });

    test('fromJson / toJson roundtrip（snake_case 键）', () {
      const state = AuthState(
        loggedIn: true,
        secretConfigured: true,
        callbackPort: 8888,
      );

      final json = state.toJson();
      expect(json['logged_in'], true);
      expect(json['secret_configured'], true);
      expect(json['callback_port'], 8888);

      final restored = AuthState.fromJson(json);
      expect(restored.loggedIn, true);
      expect(restored.secretConfigured, true);
      expect(restored.callbackPort, 8888);
    });

    test('fromJson tolerates callback_port as string', () {
      final state = AuthState.fromJson({'callback_port': '7777'});

      expect(state.callbackPort, 7777);
    });

    test('copyWith replaces specified fields', () {
      const state = AuthState();

      final copy = state.copyWith(loggedIn: true, callbackPort: 10000);

      expect(copy.loggedIn, isTrue);
      expect(copy.secretConfigured, isFalse);
      expect(copy.callbackPort, 10000);
    });
  });
}
