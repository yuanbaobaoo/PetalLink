import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/app/theme/mate_tokens.dart';
import 'mate_icon.dart';

// =============================================================================
// MateForm —— 表单控件集合（对标 CMP mate/MateForm.kt v2）。
//
// 填充式无边框：bg-fill 常态；focus→白底 + 2dp brand-light 描边；
// error→2dp error 描边；disabled 降透明度。无 ripple。
// =============================================================================

/// 自绘容器文本输入（v2：填充式无边框）。
///
/// h38，radius 8，bg=bgFill 无边框；focus→白底 + 2dp brandLight 描边；
/// error→2dp error 描边；disabled alpha 0.6；prefix 图标 textPlaceholder。
///
/// 示例：
/// ```dart
/// MateTextField(
///   controller: _controller,
///   placeholder: '请输入',
///   prefixIcon: 'search',
///   onChanged: (v) => print(v),
/// )
/// ```
class MateTextField extends StatefulWidget {
  /// 文本控制器（外部可控；不传则内部自管）。
  final TextEditingController? controller;

  /// 初始值（未传 controller 时生效）。
  final String? initialValue;

  /// 占位提示。
  final String placeholder;

  /// 是否启用。
  final bool enabled;

  /// 前缀图标 name（可选）。
  final String? prefixIcon;

  /// 错误态（红色边框）。
  final bool error;

  /// 是否密码输入。
  final bool obscureText;

  /// 键盘类型。
  final TextInputType? keyboardType;

  /// 文字变化回调。
  final ValueChanged<String>? onChanged;

  /// 提交回调（回车）。
  final ValueChanged<String>? onSubmit;

  /// 是否自动聚焦。
  final bool autofocus;

  /// 右侧自定义内容（如清除按钮）。
  final Widget? suffix;

  const MateTextField({
    super.key,
    this.controller,
    this.initialValue,
    this.placeholder = '',
    this.enabled = true,
    this.prefixIcon,
    this.error = false,
    this.obscureText = false,
    this.keyboardType,
    this.onChanged,
    this.onSubmit,
    this.autofocus = false,
    this.suffix,
  });

  @override
  State<MateTextField> createState() => _MateTextFieldState();
}

class _MateTextFieldState extends State<MateTextField> {
  late final TextEditingController _controller;
  late final FocusNode _focusNode;
  bool _focused = false;

  TextEditingController get _effectiveController =>
      widget.controller ?? _controller;

  @override
  void initState() {
    super.initState();
    _controller = TextEditingController(text: widget.initialValue ?? '');
    _focusNode = FocusNode();
    _focusNode.addListener(_onFocusChange);
  }

  @override
  void dispose() {
    if (widget.controller == null) _controller.dispose();
    _focusNode.removeListener(_onFocusChange);
    _focusNode.dispose();
    super.dispose();
  }

  void _onFocusChange() {
    setState(() => _focused = _focusNode.hasFocus);
  }

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context).form;
    final metrics = MateTheme.metricsOf(context).form;

    // focus/error 时白底 + 2dp 描边；常态透明 2dp 占位避免尺寸跳动
    final borderColor = !widget.enabled
        ? Colors.transparent
        : widget.error
            ? colors.error
            : _focused
                ? colors.brandLight
                : Colors.transparent;

    return Opacity(
      opacity: widget.enabled ? 1 : metrics.controls.textFieldDisabledAlpha,
      child: AnimatedContainer(
        duration: const Duration(milliseconds: 150),
        height: metrics.textFieldHeight,
        decoration: BoxDecoration(
          color: (_focused || widget.error)
              ? colors.bgContainer
              : colors.bgFill,
          borderRadius: BorderRadius.circular(metrics.textFieldRadius),
          border: Border.all(
            color: borderColor,
            width: metrics.controls.textFieldBorderWidth,
          ),
        ),
        padding: EdgeInsets.symmetric(
          horizontal: metrics.controls.textFieldHorizontalPadding,
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.center,
          children: [
            if (widget.prefixIcon != null) ...[
              MateIcon(
                name: widget.prefixIcon!,
                size: metrics.controls.textFieldPrefixIconSize,
                tint: colors.textPlaceholder,
              ),
              SizedBox(width: metrics.controls.textFieldContentSpacing),
            ],
            Expanded(
              child: TextField(
                controller: _effectiveController,
                focusNode: _focusNode,
                enabled: widget.enabled,
                obscureText: widget.obscureText,
                keyboardType: widget.keyboardType,
                autofocus: widget.autofocus,
                onChanged: widget.onChanged,
                onSubmitted: widget.onSubmit,
                maxLines: 1,
                style: typography.textFieldInput.copyWith(
                  color: colors.textPrimary,
                ),
                cursorColor: colors.brand,
                decoration: InputDecoration(
                  hintText: widget.placeholder,
                  hintStyle: typography.textFieldPlaceholder.copyWith(
                    color: colors.textPlaceholder,
                  ),
                  border: InputBorder.none,
                  isDense: true,
                  contentPadding: EdgeInsets.zero,
                ),
              ),
            ),
            if (widget.suffix != null) ...[
              SizedBox(width: metrics.controls.textFieldContentSpacing),
              widget.suffix!,
            ],
          ],
        ),
      ),
    );
  }
}

/// 数值输入（v2：填充式无边框，默认内容宽 120）。
///
/// 居中数字 + 可选单位后缀；min/max clamp；h38，radius 8，bgFill；
/// focus→白底 + brandLight 描边。
class MateNumberField extends StatefulWidget {
  /// 当前数值。
  final int value;

  /// 数值变化（解析失败不触发；自动 clamp 到 [min]/[max]）。
  final ValueChanged<int> onChanged;

  /// 最小值。
  final int min;

  /// 最大值。
  final int max;

  /// 单位后缀（如 "秒"）。
  final String suffix;

  /// 是否启用。
  final bool enabled;

  const MateNumberField({
    super.key,
    required this.value,
    required this.onChanged,
    this.min = 0,
    this.max = 999999,
    this.suffix = '',
    this.enabled = true,
  });

  @override
  State<MateNumberField> createState() => _MateNumberFieldState();
}

class _MateNumberFieldState extends State<MateNumberField> {
  late final TextEditingController _controller;
  late final FocusNode _focusNode;
  bool _focused = false;

  @override
  void initState() {
    super.initState();
    _controller = TextEditingController(text: widget.value.toString());
    _focusNode = FocusNode();
    _focusNode.addListener(_onFocusChange);
  }

  @override
  void didUpdateWidget(covariant MateNumberField oldWidget) {
    super.didUpdateWidget(oldWidget);
    // 外部值变化时同步文本（用户输入中不打扰）
    if (oldWidget.value != widget.value &&
        _controller.text != widget.value.toString()) {
      _controller.text = widget.value.toString();
    }
  }

  @override
  void dispose() {
    _controller.dispose();
    _focusNode.removeListener(_onFocusChange);
    _focusNode.dispose();
    super.dispose();
  }

  void _onFocusChange() {
    setState(() => _focused = _focusNode.hasFocus);
  }

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context).form;
    final metrics = MateTheme.metricsOf(context).form;

    return AnimatedContainer(
      duration: const Duration(milliseconds: 150),
      height: metrics.numberFieldHeight,
      decoration: BoxDecoration(
        color: _focused ? colors.bgContainer : colors.bgFill,
        borderRadius: BorderRadius.circular(metrics.numberFieldRadius),
        border: Border.all(
          color: _focused ? colors.brandLight : Colors.transparent,
          width: metrics.controls.numberFieldBorderWidth,
        ),
      ),
      padding: EdgeInsets.symmetric(
        horizontal: metrics.controls.numberFieldHorizontalPadding,
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          SizedBox(
            width: metrics.controls.numberFieldInputWidth,
            child: TextField(
              controller: _controller,
              focusNode: _focusNode,
              enabled: widget.enabled,
              keyboardType: TextInputType.number,
              maxLines: 1,
              textAlign: TextAlign.center,
              style: typography.numberFieldInput.copyWith(
                color: colors.textPrimary,
              ),
              cursorColor: colors.brand,
              decoration: const InputDecoration(
                border: InputBorder.none,
                isDense: true,
                contentPadding: EdgeInsets.zero,
              ),
              onChanged: (input) {
                // 仅保留数字与负号；解析失败不回调；自动 clamp 到 [min, max]
                final filtered =
                    input.replaceAll(RegExp(r'[^0-9-]'), '');
                if (filtered != input) _controller.text = filtered;
                final parsed = int.tryParse(filtered);
                if (parsed != null) {
                  final clamped = parsed.clamp(widget.min, widget.max);
                  if (clamped != widget.value) widget.onChanged(clamped);
                }
              },
            ),
          ),
          if (widget.suffix.isNotEmpty) ...[
            SizedBox(width: metrics.controls.numberFieldContentSpacing),
            Text(
              widget.suffix,
              style: typography.numberFieldSuffix.copyWith(
                color: colors.textSecondary,
              ),
            ),
          ],
        ],
      ),
    );
  }
}

/// 步进器（v2：容器嵌入式）。
///
/// 外层 bgFill 容器（radius 8，padding 3）；[−|值|+] 按钮 30×30 radius 5，
/// hover 白底 + 柔影 + brand 字；disabled textPlaceholder；
/// minus 用 x 图标 rotate 45° 变减号，plus 字号 16 medium；中间 value 宽 44 居中。
class MateStepper extends StatelessWidget {
  /// 当前值。
  final int value;

  /// 值变化回调。
  final ValueChanged<int> onChanged;

  /// 最小值。
  final int min;

  /// 最大值。
  final int max;

  /// 步长。
  final int step;

  const MateStepper({
    super.key,
    required this.value,
    required this.onChanged,
    this.min = 0,
    this.max = 999999,
    this.step = 1,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final controls = MateTheme.metricsOf(context).form.controls;
    final typography = MateTheme.typographyOf(context).form;

    final minusEnabled = value > min;
    final plusEnabled = value < max;

    return Container(
      decoration: BoxDecoration(
        color: colors.bgFill,
        borderRadius: BorderRadius.circular(controls.stepperRadius),
      ),
      padding: EdgeInsets.all(controls.stepperPadding),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          _StepperButton(
            enabled: minusEnabled,
            onClick: () => onChanged((value - step).clamp(min, max)),
            contentBuilder: (color) => Transform.rotate(
              // 用 x 旋转 45 度作为减号（与原 Vue 一致）
              angle: 3.141592653589793 / 4,
              child: Transform.scale(
                scale: 0.8,
                child: MateIcon(
                  name: 'x',
                  size: controls.stepperMinusIconSize,
                  tint: color,
                ),
              ),
            ),
          ),
          SizedBox(
            width: controls.stepperValueWidth,
            child: Center(
              child: Text(
                value.toString(),
                style: typography.stepperValue.copyWith(
                  color: colors.textPrimary,
                ),
              ),
            ),
          ),
          _StepperButton(
            enabled: plusEnabled,
            onClick: () => onChanged((value + step).clamp(min, max)),
            contentBuilder: (color) => Text(
              '+',
              style: typography.stepperAction.copyWith(color: color),
            ),
          ),
        ],
      ),
    );
  }
}

/// 步进器按钮（30×30，hover 白底柔影）。
class _StepperButton extends StatefulWidget {
  final bool enabled;
  final VoidCallback onClick;
  final Widget Function(Color color) contentBuilder;

  const _StepperButton({
    required this.enabled,
    required this.onClick,
    required this.contentBuilder,
  });

  @override
  State<_StepperButton> createState() => _StepperButtonState();
}

class _StepperButtonState extends State<_StepperButton> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final controls = MateTheme.metricsOf(context).form.controls;
    final active = widget.enabled && _hovered;

    return MouseRegion(
      cursor: widget.enabled
          ? SystemMouseCursors.click
          : SystemMouseCursors.basic,
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: GestureDetector(
        onTap: widget.enabled ? widget.onClick : null,
        child: Opacity(
          opacity: widget.enabled ? 1 : controls.stepperDisabledAlpha,
          child: AnimatedContainer(
            duration: const Duration(milliseconds: 150),
            width: controls.stepperButtonSize,
            height: controls.stepperButtonSize,
            decoration: BoxDecoration(
              color: active ? colors.bgContainer : Colors.transparent,
              borderRadius: BorderRadius.circular(controls.stepperButtonRadius),
              boxShadow: active
                  ? [
                      BoxShadow(
                        color: MateColors.controlShadowSoft,
                        blurRadius: controls.stepperButtonShadowElevation * 2,
                        offset: Offset(0, controls.stepperButtonShadowElevation),
                      ),
                    ]
                  : null,
            ),
            alignment: Alignment.center,
            child: widget.contentBuilder(
              active ? colors.brand : colors.textSecondary,
            ),
          ),
        ),
      ),
    );
  }
}

/// 搜索框（v2：填充式，h38）。
///
/// 内嵌 [MateTextField] + search 前缀图标。
class MateSearchField extends StatelessWidget {
  /// 当前关键词。
  final String? value;

  /// 文本控制器（外部可控）。
  final TextEditingController? controller;

  /// 关键词变化回调。
  final ValueChanged<String>? onChanged;

  /// 占位提示。
  final String placeholder;

  /// 回车提交（传当前关键词）。
  final ValueChanged<String>? onSubmit;

  /// 是否自动聚焦。
  final bool autofocus;

  const MateSearchField({
    super.key,
    this.value,
    this.controller,
    this.onChanged,
    this.placeholder = '搜索文件和文件夹...',
    this.onSubmit,
    this.autofocus = false,
  });

  @override
  Widget build(BuildContext context) {
    final metrics = MateTheme.metricsOf(context).form;
    return SizedBox(
      height: metrics.searchFieldHeight,
      child: MateTextField(
        controller: controller,
        initialValue: value,
        placeholder: placeholder,
        prefixIcon: 'search',
        onChanged: onChanged,
        onSubmit: onSubmit,
        autofocus: autofocus,
      ),
    );
  }
}

/// 开关（v2：iOS 风格 46×28）。
///
/// 46×28，radius full，bg=switchOffTrack(off)/brand(on)，过渡 0.2s；
/// knob 22×22 白圆，off 靠左 3，on 偏移 21；disabled alpha 0.5。
class MateSwitch extends StatelessWidget {
  /// 开关状态。
  final bool checked;

  /// 状态变化回调。
  final ValueChanged<bool>? onChanged;

  /// 是否禁用。
  final bool disabled;

  const MateSwitch({
    super.key,
    required this.checked,
    this.onChanged,
    this.disabled = false,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final controls = MateTheme.metricsOf(context).form.controls;

    final trackColor = checked ? colors.brand : colors.switchOffTrack;
    final knobOffset = checked
        ? controls.switchCheckedKnobOffset
        : controls.switchUncheckedKnobOffset;

    return MouseRegion(
      cursor: disabled ? SystemMouseCursors.basic : SystemMouseCursors.click,
      child: GestureDetector(
        onTap: disabled ? null : () => onChanged?.call(!checked),
        child: Opacity(
          opacity: disabled ? controls.switchDisabledAlpha : 1,
          child: AnimatedContainer(
            duration: const Duration(milliseconds: 200),
            width: controls.switchWidth,
            height: controls.switchHeight,
            decoration: BoxDecoration(
              color: trackColor,
              borderRadius: BorderRadius.circular(controls.switchRadius),
            ),
            child: AnimatedPadding(
              duration: const Duration(milliseconds: 200),
              padding: EdgeInsets.only(left: knobOffset),
              child: Align(
                alignment: Alignment.centerLeft,
                child: Container(
                  width: controls.switchKnobSize,
                  height: controls.switchKnobSize,
                  decoration: BoxDecoration(
                    color: colors.switchKnob,
                    shape: BoxShape.circle,
                    boxShadow: [
                      BoxShadow(
                        color: MateColors.controlShadowStrong,
                        blurRadius: controls.switchKnobShadowElevation * 2,
                        offset: Offset(0, controls.switchKnobShadowElevation / 2),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// 复选框（v2：18px 圆角矩形，支持 tri-state）。
///
/// 1.5px border，radius 5，bgContainer；hover border→brand；
/// active bg=brand border=brand，显示白色 check 图标；
/// 半选（null）显示 1.5px 高白条；disabled alpha 0.5。
/// 点击循环：null→true→false→null。
class MateCheckbox extends StatefulWidget {
  /// 三态：true/false/null(半选)。
  final bool? checked;

  /// 状态变化（tri-state 循环 null→true→false→null）。
  final ValueChanged<bool?>? onChanged;

  /// 尺寸（默认 18）。
  final double? size;

  /// 是否禁用。
  final bool disabled;

  const MateCheckbox({
    super.key,
    required this.checked,
    this.onChanged,
    this.size,
    this.disabled = false,
  });

  @override
  State<MateCheckbox> createState() => _MateCheckboxState();
}

class _MateCheckboxState extends State<MateCheckbox> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final controls = MateTheme.metricsOf(context).form.controls;

    final size = widget.size ?? controls.checkboxDefaultSize;
    final isChecked = widget.checked == true;
    final isIndeterminate = widget.checked == null;
    final borderColor = isChecked || isIndeterminate
        ? colors.brand
        : _hovered && !widget.disabled
            ? colors.brand
            : colors.textPlaceholder;
    final bgColor =
        isChecked || isIndeterminate ? colors.brand : colors.bgContainer;

    return MouseRegion(
      cursor: widget.disabled
          ? SystemMouseCursors.basic
          : SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: GestureDetector(
        onTap: widget.disabled
            ? null
            : () {
                // tri-state 循环：null→true→false→null
                final next = switch (widget.checked) {
                  null => true,
                  true => false,
                  false => null,
                };
                widget.onChanged?.call(next);
              },
        child: Opacity(
          opacity: widget.disabled ? controls.checkboxDisabledAlpha : 1,
          child: AnimatedContainer(
            duration: const Duration(milliseconds: 150),
            width: size,
            height: size,
            decoration: BoxDecoration(
              color: bgColor,
              borderRadius: BorderRadius.circular(controls.checkboxRadius),
              border: Border.all(
                color: borderColor,
                width: controls.checkboxBorderWidth,
              ),
            ),
            alignment: Alignment.center,
            child: isChecked
                ? MateIcon(
                    name: 'check',
                    size: size - controls.checkboxCheckInset,
                    tint: colors.checkboxMark,
                  )
                : isIndeterminate
                    ? Container(
                        width: size - controls.checkboxIndeterminateInset,
                        height: controls.checkboxIndeterminateHeight,
                        decoration: BoxDecoration(
                          color: colors.checkboxMark,
                          borderRadius: BorderRadius.circular(
                            controls.checkboxIndeterminateRadius,
                          ),
                        ),
                      )
                    : null,
          ),
        ),
      ),
    );
  }
}

/// 单选项（对标原 Vue `<MateRadio>`）。圆形，选中显示 brand 实心圆点。
class MateRadio extends StatelessWidget {
  /// 是否选中。
  final bool selected;

  /// 选中回调。
  final VoidCallback? onSelect;

  /// 尺寸（默认 16）。
  final double? size;

  /// 是否禁用。
  final bool disabled;

  const MateRadio({
    super.key,
    required this.selected,
    this.onSelect,
    this.size,
    this.disabled = false,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final controls = MateTheme.metricsOf(context).form.controls;
    final size = this.size ?? controls.radioDefaultSize;
    final borderColor = selected ? colors.brand : colors.border;

    return MouseRegion(
      cursor: disabled ? SystemMouseCursors.basic : SystemMouseCursors.click,
      child: GestureDetector(
        onTap: disabled ? null : onSelect,
        child: Opacity(
          opacity: disabled ? controls.radioDisabledAlpha : 1,
          child: Container(
            width: size,
            height: size,
            decoration: BoxDecoration(
              color: colors.bgContainer,
              shape: BoxShape.circle,
              border: Border.all(
                color: borderColor,
                width: controls.radioBorderWidth,
              ),
            ),
            alignment: Alignment.center,
            child: selected
                ? Container(
                    width: size * 0.5,
                    height: size * 0.5,
                    decoration: BoxDecoration(
                      color: colors.brand,
                      shape: BoxShape.circle,
                    ),
                  )
                : null,
          ),
        ),
      ),
    );
  }
}
