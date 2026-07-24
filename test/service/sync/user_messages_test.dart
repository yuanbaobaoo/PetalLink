import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/service/sync/user_messages.dart';

/// simplifySyncError 术语转换合同测试
/// （对齐 Rust `user_messages.rs` 的 `simplifies_replan_terms` 与 `preserves_plain_errors`）。

void main() {
  group('simplifySyncError', () {
    test('重新规划类技术错误 → 转换为用户可执行建议', () {
      // 对齐 Rust simplifies_replan_terms
      expect(
        simplifySyncError('远端文件已在规划后变化，拒绝用旧任务覆盖'),
        '云端文件已更新。为避免覆盖，请同步索引后重试。',
      );
      expect(
        simplifySyncError('本地上传源已变化，需要重新规划'),
        '本地文件已发生变化，请重新检查并重试。',
      );
    });

    test('已清晰的普通错误 → 原样保留，不被泛化覆盖', () {
      // 对齐 Rust preserves_plain_errors
      expect(simplifySyncError('网络连接失败'), '网络连接失败');
    });

    test('文件被占用 → 提示关闭后自动继续', () {
      expect(
        simplifySyncError('用户正在编辑该文件'),
        '文件正在编辑，保存并关闭后会自动继续。',
      );
    });

    test('续传信息失效 → 提示重新上传', () {
      expect(
        simplifySyncError('session_url 已过期，无法安全重放'),
        '续传信息已失效，请重新开始上传。',
      );
    });

    test('释放空间无基线 → 提示暂无法释放', () {
      expect(
        simplifySyncError('找不到与路径匹配的成功同步基线'),
        '没有找到可用于核对的同步记录，暂时无法释放空间。',
      );
    });

    test('云端版本快照缺失 → 提示信息不完整', () {
      expect(
        simplifySyncError('更新上传缺少云端版本快照，需要重新规划'),
        '文件同步信息不完整，请同步索引后重试。',
      );
    });

    test('远端核验进行中 → 提示稍后查看（非失败语义）', () {
      expect(
        simplifySyncError('远端核验暂不可用'),
        '正在确认同步结果，请稍后查看。',
      );
    });

    test('空字符串 → 原样返回', () {
      expect(simplifySyncError(''), '');
    });
  });
}
