import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:get/get.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/pages/settings/controller/settings_controller.dart';
import 'package:petal_link/pages/settings/widgets/settings_panels.dart';
import 'package:petal_link/widgets/index.dart';

/// 传输设置分区（对标 CMP SettingsScreen.kt TRANSFER 分支）。
///
/// 「传输参数」（并发上传数 / Debounce 时长 / 自动同步间隔）
/// +「同步过滤」（跳过文件逗号分隔输入框）。
class TransferSection extends StatefulWidget {
  /// 页面控制器
  final SettingsController notifier;

  /// 当前状态
  final SettingsState state;

  const TransferSection({
    super.key,
    required this.notifier,
    required this.state,
  });

  @override
  State<TransferSection> createState() => _TransferSectionState();
}

class _TransferSectionState extends State<TransferSection> {
  /// 跳过文件输入框控制器（逗号分隔文本 ↔ skipPatterns 列表双向同步）
  late final TextEditingController _skipCtrl;

  /// 监听外部状态变化（如「重置默认」/ 导入配置）同步输入框文本
  late final Worker _syncWorker;

  @override
  void initState() {
    super.initState();
    _skipCtrl = TextEditingController(
      text: widget.state.skipPatterns.join(', '),
    );
    _syncWorker = ever(widget.notifier.state, (SettingsState s) {
      // 仅当变化来自外部（非本输入框驱动）时覆盖文本，避免打扰输入
      if (!listEquals(_parsePatterns(_skipCtrl.text), s.skipPatterns)) {
        _skipCtrl.text = s.skipPatterns.join(', ');
      }
    });
  }

  @override
  void dispose() {
    _syncWorker.dispose();
    _skipCtrl.dispose();
    super.dispose();
  }

  /// 逗号分隔文本 → 模式列表（trim + 去空，对齐 CMP split 语义）
  static List<String> _parsePatterns(String text) {
    return text
        .split(',')
        .map((e) => e.trim())
        .where((e) => e.isNotEmpty)
        .toList();
  }

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).settings;
    final typography = MateTheme.typographyOf(context).settings;
    final state = widget.state;
    final notifier = widget.notifier;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const MateSectionHeader(text: '传输设置', icon: 'transfer'),
        SettingsPanel(
          children: [
            const SettingsGroupHeader('传输参数', first: true),
            SettingRow(
              label: '并发上传数',
              desc: '同时进行的文件传输任务数量。较高值可提升大文件传输效率，但会占用更多网络带宽。',
              control: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  MateStepper(
                    value: state.concurrency,
                    onChanged: notifier.setConcurrency,
                    min: 1,
                    max: 20,
                  ),
                  SizedBox(width: metrics.concurrencyContentSpacing),
                  Text(
                    '范围 1-20',
                    style: typography.numberRangeHint.copyWith(
                      color: colors.textSecondary,
                    ),
                  ),
                ],
              ),
            ),
            SettingRow(
              label: 'Debounce 时长',
              desc: '文件变更后等待多少秒再触发同步上传，避免频繁修改导致重复传输。',
              control: MateNumberField(
                value: state.debounce,
                onChanged: notifier.setDebounce,
                min: 1,
                max: 600,
                suffix: '秒',
              ),
            ),
            SettingRow(
              label: '自动同步间隔',
              desc: '每隔多久自动从云端拉取最新变更（新增/修改/删除）。0 = 关闭自动同步，仅手动点「同步索引」。设为 60 以上时生效。',
              control: MateNumberField(
                value: state.pollInterval,
                onChanged: notifier.setPollInterval,
                min: 0,
                max: 86400,
                suffix: '秒',
              ),
            ),
            const SettingsGroupHeader('同步过滤'),
            SettingRow(
              label: '跳过文件（逗号分隔）',
              desc: '匹配名称的文件不会被同步，如 .DS_Store、临时文件。',
              showDivider: false,
              control: SizedBox(
                width: metrics.skipPatternFieldWidth,
                child: MateTextField(
                  controller: _skipCtrl,
                  placeholder: '.DS_Store, .tmp, ~\$*, .Trash',
                  onChanged: (v) =>
                      notifier.setSkipPatterns(_parsePatterns(v)),
                ),
              ),
            ),
          ],
        ),
      ],
    );
  }
}
