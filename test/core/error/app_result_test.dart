import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';

void main() {
  group('AppResult<T> sealed class', () {
    group('Ok', () {
      test('isOk returns true', () {
        const result = Ok<int>(42);
        expect(result.isOk, isTrue);
        expect(result.isErr, isFalse);
      });

      test('unwrap() returns the value', () {
        const result = Ok<String>('hello');
        expect(result.unwrap(), 'hello');
      });

      test('unwrapOr() returns the value ignoring default', () {
        const result = Ok<int>(10);
        expect(result.unwrapOr(99), 10);
      });

      test('map() transforms the value', () {
        const result = Ok<int>(5);
        final mapped = result.map((v) => (v * 2).toString());

        expect(mapped, isA<Ok<String>>());
        expect((mapped as Ok<String>).value, '10');
      });

      test('map() to different type works', () {
        const result = Ok<String>('hello');
        final mapped = result.map((v) => v.length);

        expect(mapped, isA<Ok<int>>());
        expect((mapped as Ok<int>).value, 5);
      });

      test('fold() calls onOk branch', () {
        const result = Ok<String>('success');
        final folded = result.fold(
          onOk: (v) => 'OK: $v',
          onErr: (e) => 'ERR: ${e.message}',
        );

        expect(folded, 'OK: success');
      });

      test('fold() does not call onErr', () {
        const result = Ok<int>(1);
        var errCalled = false;

        result.fold(
          onOk: (v) => v * 2,
          onErr: (e) {
            errCalled = true;
            return -1;
          },
        );

        expect(errCalled, isFalse);
      });

      test('== operator works for equal values', () {
        expect(const Ok<int>(1), const Ok<int>(1));
        expect(const Ok<String>('a'), const Ok<String>('a'));
      });

      test('== operator works for unequal values', () {
        expect(const Ok<int>(1), isNot(const Ok<int>(2)));
      });

      test('toString() includes value', () {
        expect(const Ok<int>(42).toString(), 'Ok(42)');
        expect(const Ok<String>('hello').toString(), 'Ok(hello)');
      });
    });

    group('Err', () {
      test('isErr returns true', () {
        final result = Err<String>(const GenericError(message: 'fail'));
        expect(result.isErr, isTrue);
        expect(result.isOk, isFalse);
      });

      test('unwrap() throws the contained AppError', () {
        final error = const GenericError(message: 'test error');
        final result = Err<String>(error);

        expect(
          () => result.unwrap(),
          throwsA(isA<GenericError>()),
        );
      });

      test('unwrap() throws the specific error instance', () {
        final error = const AuthError(authCode: AuthErrorCode.denied, message: 'auth failed');
        final result = Err<int>(error);

        expect(
          () => result.unwrap(),
          throwsA(isA<AuthError>().having(
            (e) => e.message,
            'message',
            'auth failed',
          )),
        );
      });

      test('unwrapOr() returns the default value', () {
        final result = Err<int>(const GenericError(message: 'fail'));
        expect(result.unwrapOr(42), 42);
      });

      test('unwrapOr() returns default of different type for type inference', () {
        final result = Err<String>(const GenericError(message: 'fail'));
        expect(result.unwrapOr('default'), 'default');
      });

      test('map() preserves the Err unchanged', () {
        final error = const DriveApiError(driveCode: DriveApiErrorCode.fromStatus, message: 'server error', statusCode: 500);
        final result = Err<int>(error);

        final mapped = result.map((v) => v * 2);

        expect(mapped, isA<Err<int>>());
        expect((mapped as Err<int>).error, same(error));
      });

      test('map() does not call transform function', () {
        final result = Err<int>(const GenericError(message: 'fail'));
        var called = false;

        result.map((v) {
          called = true;
          return v;
        });

        expect(called, isFalse);
      });

      test('fold() calls onErr branch', () {
        final error = const GenericError(message: 'network down');
        final result = Err<String>(error);

        final folded = result.fold(
          onOk: (v) => 'OK: $v',
          onErr: (e) => 'ERR: ${e.message}',
        );

        expect(folded, 'ERR: network down');
      });

      test('fold() does not call onOk', () {
        final result = Err<int>(const GenericError(message: 'fail'));
        var okCalled = false;

        result.fold(
          onOk: (v) {
            okCalled = true;
            return v;
          },
          onErr: (e) => -1,
        );

        expect(okCalled, isFalse);
      });
    });

    group('type inference', () {
      test('Ok and Err can be used in the same list as AppResult', () {
        final List<AppResult<int>> results = [
          const Ok<int>(1),
          Err<int>(const GenericError(message: 'fail')),
        ];

        expect(results[0].isOk, isTrue);
        expect(results[1].isErr, isTrue);
      });

      test('map can change type parameter', () {
        final result = Ok<int>(1).map((v) => v.toString());
        expect(result, isA<Ok<String>>());
      });
    });
  });
}
