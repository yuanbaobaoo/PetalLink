@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.components.mate

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.hoverable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.draw.scale
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.theme.LOCAL_SEMANTIC_COLORS
import io.github.yuanbaobaoo.petallink.ui.theme.PetalTheme

/**
 * 自绘容器文本输入（v2：填充式无边框）。
 *
 * h38，radius-md(8)，bg=bg-fill 无边框；focus→白底 + 2dp brand-light 描边；error→2dp error 描边；
 * disabled bg-fill opacity 0.6；prefix 图标 text-placeholder；placeholder text-placeholder。
 *
 * @param value 当前文本
 * @param onValueChange 文本变化回调
 * @param placeholder 占位提示
 * @param modifier 外部 Modifier
 * @param enabled 是否启用
 * @param prefixIcon 前缀图标 name（可选）
 * @param error 错误态（红色边框）
 * @param singleLine 单行（默认 true）
 * @param suffix 右侧自定义内容（如清除按钮）
 */
@Composable
fun MateTextField(
    value: String,
    onValueChange: (String) -> Unit,
    placeholder: String = "",
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    prefixIcon: String? = null,
    error: Boolean = false,
    singleLine: Boolean = true,
    keyboardOptions: KeyboardOptions = KeyboardOptions.Default,
    visualTransformation: VisualTransformation = VisualTransformation.None,
    suffix: @Composable (() -> Unit)? = null,
) {
    val interaction = remember { MutableInteractionSource() }
    val focused by interaction.collectIsFocusedAsState()
    val semantic = LOCAL_SEMANTIC_COLORS.current
    val formTypography = PetalTheme.typography.form
    val formMetrics = PetalTheme.metrics.form
    // focus/error 时白底 + 2dp 描边；常态 bg-fill 无描边（用透明 2dp 占位避免尺寸跳动）
    val borderColor = when {
        !enabled -> Color.Transparent
        error -> PetalTheme.colors.error
        focused -> PetalTheme.colors.brandLight
        else -> Color.Transparent
    }
    Row(
        modifier = modifier
            .height(formMetrics.textFieldHeight)
            .clip(RoundedCornerShape(formMetrics.textFieldRadius))
            .background(if (focused || error) semantic.bgContainer else semantic.bgFill)
            .border(width = formMetrics.controls.textFieldBorderWidth, color = borderColor, shape = RoundedCornerShape(formMetrics.textFieldRadius))
            .alpha(if (enabled) 1f else formMetrics.controls.textFieldDisabledAlpha)
            .padding(horizontal = formMetrics.controls.textFieldHorizontalPadding),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(formMetrics.controls.textFieldContentSpacing),
    ) {
        if (prefixIcon != null) {
            MateIcon(name = prefixIcon, size = formMetrics.controls.textFieldPrefixIconSize, tint = semantic.textPlaceholder)
        }
        Box(modifier = Modifier.weight(1f)) {
            // BasicTextField 无 ripple，完全自绘光标
            BasicTextField(
                value = value,
                onValueChange = onValueChange,
                modifier = Modifier.fillMaxWidth(),
                enabled = enabled,
                singleLine = singleLine,
                textStyle = formTypography.textFieldInput.copy(color = semantic.textPrimary),
                cursorBrush = SolidColor(PetalTheme.colors.brand),
                interactionSource = interaction,
                keyboardOptions = keyboardOptions,
                visualTransformation = visualTransformation,
            )
            if (value.isEmpty() && placeholder.isNotEmpty()) {
                Text(
                    placeholder,
                    color = semantic.textPlaceholder,
                    style = formTypography.textFieldPlaceholder,
                )
            }
        }
        suffix?.invoke()
    }
}

/**
 * 数值输入（v2：填充式无边框，默认内容宽 120）。
 *
 * 居中数字 + 可选单位后缀；min/max clamp；h38，radius-md(8)，bg-fill；focus→白底 + brand-light 描边。
 * 隐藏原生 spin button（BasicTextField 无 spin）。
 * 内容区固定 120dp（v2 .number-field .input w120），不使用 weight 填充——
 * 避免在左右布局的设置行内把文本区挤压成竖排。
 *
 * @param value 当前数值
 * @param onValueChange 数值变化（NaN 不触发；自动 clamp 到 [min]/[max]）
 * @param min 最小值
 * @param max 最大值
 * @param suffix 单位后缀（如 "秒"）
 * @param enabled 是否启用
 */
@Composable
fun MateNumberField(
    value: Int,
    onValueChange: (Int) -> Unit,
    modifier: Modifier = Modifier,
    min: Int = 0,
    max: Int = 999_999,
    suffix: String = "",
    enabled: Boolean = true,
) {
    var text by remember(value) { mutableStateOf(value.toString()) }
    val semantic = LOCAL_SEMANTIC_COLORS.current
    val interaction = remember { MutableInteractionSource() }
    val focused by interaction.collectIsFocusedAsState()
    val formTypography = PetalTheme.typography.form
    val formMetrics = PetalTheme.metrics.form

    Row(
        modifier = modifier
            .height(formMetrics.numberFieldHeight)
            .clip(RoundedCornerShape(formMetrics.numberFieldRadius))
            .background(if (focused) semantic.bgContainer else semantic.bgFill)
            .border(formMetrics.controls.numberFieldBorderWidth, if (focused) PetalTheme.colors.brandLight else Color.Transparent, RoundedCornerShape(formMetrics.numberFieldRadius))
            .padding(horizontal = formMetrics.controls.numberFieldHorizontalPadding),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(formMetrics.controls.numberFieldContentSpacing),
    ) {
        Box(modifier = Modifier.width(formMetrics.controls.numberFieldInputWidth)) {
            BasicTextField(
                value = text,
                onValueChange = { input ->
                    text = input.filter { it.isDigit() || it == '-' }
                    text.toIntOrNull()?.let { parsed ->
                        // clamp 到 [min, max]；NaN 不回调
                        val clamped = parsed.coerceIn(min, max)
                        if (clamped != value) onValueChange(clamped)
                    }
                },
                modifier = Modifier.fillMaxWidth(),
                enabled = enabled,
                singleLine = true,
                textStyle = formTypography.numberFieldInput.copy(
                    color = semantic.textPrimary,
                    textAlign = TextAlign.Center,
                ),
                cursorBrush = SolidColor(PetalTheme.colors.brand),
                interactionSource = interaction,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
            )
        }
        if (suffix.isNotEmpty()) {
            Text(suffix, color = semantic.textSecondary, style = formTypography.numberFieldSuffix)
        }
    }
}

/**
 * 步进器（v2：容器嵌入式）。
 *
 * 外层 bg-fill 容器（radius-md 8，padding 3）；[−|值|+] 按钮 30×30 radius-sm(5)，
 * hover 白底 + 柔影 + brand 字；disabled text-placeholder；
 * minus 用 x 图标 rotate 45° 变减号，plus 字号 18px；中间 value 宽 44 居中 medium。
 *
 * @param value 当前值
 * @param onValueChange 值变化回调
 * @param min 最小值（默认 0）
 * @param max 最大值（默认 999999）
 * @param step 步长（默认 1）
 */
@Composable
fun MateStepper(
    value: Int,
    onValueChange: (Int) -> Unit,
    modifier: Modifier = Modifier,
    min: Int = 0,
    max: Int = 999_999,
    step: Int = 1,
) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    val minusEnabled = value > min
    val plusEnabled = value < max

    Row(
        modifier = modifier
            .clip(RoundedCornerShape(PetalTheme.metrics.form.controls.stepperRadius))
            .background(semantic.bgFill)
            .padding(PetalTheme.metrics.form.controls.stepperPadding),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        StepperButton(enabled = minusEnabled, onClick = { onValueChange((value - step).coerceIn(min, max)) }) { color ->
            // 用 x 旋转 45 度作为减号（与原 Vue 一致）
            MateIcon(name = "x", size = PetalTheme.metrics.form.controls.stepperMinusIconSize, tint = color, modifier = Modifier.scale(0.8f).rotate45())
        }
        // 中间数值（宽 44，居中，medium）
        Box(
            modifier = Modifier.width(PetalTheme.metrics.form.controls.stepperValueWidth),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                value.toString(),
                color = semantic.textPrimary,
                style = PetalTheme.typography.form.stepperValue,
            )
        }
        StepperButton(enabled = plusEnabled, onClick = { onValueChange((value + step).coerceIn(min, max)) }) { color ->
            Text("+", color = color, style = PetalTheme.typography.form.stepperAction)
        }
    }
}

/**
 * 步进器按钮（30×30，hover 白底柔影）。
 */
@Composable
private fun StepperButton(enabled: Boolean, onClick: () -> Unit, content: @Composable (Color) -> Unit) {
    val interaction = remember { MutableInteractionSource() }
    val hovered by interaction.collectIsHoveredAsState()
    val semantic = LOCAL_SEMANTIC_COLORS.current
    val active = enabled && hovered
    Box(
        modifier = Modifier
            .size(PetalTheme.metrics.form.controls.stepperButtonSize)
            .shadow(
                if (active) {
                    PetalTheme.metrics.form.controls.stepperButtonShadowElevation
                } else {
                    PetalTheme.metrics.form.controls.stepperButtonIdleElevation
                },
                RoundedCornerShape(PetalTheme.metrics.form.controls.stepperButtonRadius),
            )
            .clip(RoundedCornerShape(PetalTheme.metrics.form.controls.stepperButtonRadius))
            .background(if (active) semantic.bgContainer else Color.Transparent)
            .alpha(if (enabled) 1f else PetalTheme.metrics.form.controls.stepperDisabledAlpha)
            .hoverable(interaction)
            .then(
                if (enabled) Modifier.clickable(interactionSource = interaction, indication = null, onClick = onClick)
                else Modifier,
            ),
        contentAlignment = Alignment.Center,
    ) {
        content(if (active) PetalTheme.colors.brand else semantic.textSecondary)
    }
}

/**
 * x 图标旋转 45° 近似减号的 Modifier（封装保持调用点简洁）。
 */
private fun Modifier.rotate45(): Modifier = this.scale(scaleX = 0.8f, scaleY = 0.8f).rotate(45f)

/**
 * 搜索框（v2：填充式，h38）。
 *
 * 内嵌 [MateTextField] + search 前缀图标；h38；bg=bg-fill。
 *
 * @param value 当前关键词
 * @param onValueChange 关键词变化
 * @param placeholder 占位提示（默认「搜索文件和文件夹...」）
 * @param maxWidth 最大宽度（dp，0 = 100%）
 * @param onSubmit 回车提交（传当前关键词）
 */
@Composable
fun MateSearchField(
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    placeholder: String = "搜索文件和文件夹...",
    maxWidth: Dp = PetalTheme.metrics.form.controls.searchUnboundedWidth,
    onSubmit: (String) -> Unit = {},
) {
    val widthMod = if (maxWidth > PetalTheme.metrics.form.controls.searchUnboundedWidth) Modifier.width(maxWidth) else Modifier.fillMaxWidth()
    Box(modifier = modifier.then(widthMod).height(PetalTheme.metrics.form.searchFieldHeight)) {
        MateTextField(
            value = value,
            onValueChange = onValueChange,
            placeholder = placeholder,
            modifier = Modifier.fillMaxWidth(),
            prefixIcon = "search",
            keyboardOptions = KeyboardOptions.Default.copy(
                imeAction = androidx.compose.ui.text.input.ImeAction.Search,
            ),
        )
    }
}

/**
 * 开关（v2：iOS 风格 46×28）。
 *
 * 46×28，radius full，bg=switch-off(off)/brand(on)，过渡 0.2s；
 * knob 22×22 白圆 top/left 3，box-shadow，on 时 translateX 21；disabled opacity 0.5。
 *
 * @param checked 开关状态
 * @param onCheckedChange 状态变化回调
 * @param disabled 是否禁用
 */
@Composable
fun MateSwitch(
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    disabled: Boolean = false,
) {
    val trackColor = if (checked) PetalTheme.colors.brand else PetalTheme.colors.switchOffTrack
    val controls = PetalTheme.metrics.form.controls
    val knobOffset = if (checked) controls.switchCheckedKnobOffset else controls.switchUncheckedKnobOffset
    Row(
        modifier = modifier
            .then(
                if (disabled) Modifier
                else Modifier.clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {
                    onCheckedChange(!checked)
                },
            )
            .alpha(if (disabled) PetalTheme.metrics.form.controls.switchDisabledAlpha else 1f)
            .size(controls.switchWidth, controls.switchHeight)
            .clip(RoundedCornerShape(controls.switchRadius))
            .background(trackColor),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // knob 22×22 白圆，靠左偏移 3(off) 或 21(on)
        Box(
            modifier = Modifier
                .padding(start = knobOffset)
                .size(controls.switchKnobSize)
                .shadow(controls.switchKnobShadowElevation, CircleShape)
                .clip(CircleShape)
                .background(PetalTheme.colors.switchKnob),
        )
    }
}

/**
 * 复选框（v2：18px 圆角矩形，支持 tri-state）。
 *
 * 1.5px border，radius-sm(5)，bg-container；hover border→brand；
 * active bg=brand border=brand，显示白色 check 图标；
 * 半选（null）显示 1.5px 高白条；disabled opacity 0.5。
 *
 * @param checked 三态：true/false/null(半选)
 * @param onCheckedChange 状态变化（tri-state 循环 null→true→false→null）
 * @param size 尺寸（默认 18）
 * @param disabled 是否禁用
 */
@Composable
fun MateCheckbox(
    checked: Boolean?,
    onCheckedChange: (Boolean?) -> Unit,
    modifier: Modifier = Modifier,
    size: Dp = PetalTheme.metrics.form.controls.checkboxDefaultSize,
    disabled: Boolean = false,
) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    val interaction = remember { MutableInteractionSource() }
    val hovered by interaction.collectIsFocusedAsState()
    val isChecked = checked == true
    val isIndeterminate = checked == null
    val borderColor = when {
        isChecked || isIndeterminate -> PetalTheme.colors.brand
        hovered && !disabled -> PetalTheme.colors.brand
        else -> semantic.textPlaceholder
    }
    val bgColor = if (isChecked || isIndeterminate) PetalTheme.colors.brand else semantic.bgContainer

    Box(
        modifier = modifier
            .then(
                if (disabled) Modifier
                else Modifier.clickable(interactionSource = interaction, indication = null) {
                    // tri-state 循环：null→true→false→null
                    val next = when (checked) {
                        null -> true
                        true -> false
                        false -> null
                    }
                    onCheckedChange(next)
                },
            )
            .alpha(if (disabled) PetalTheme.metrics.form.controls.checkboxDisabledAlpha else 1f)
            .size(size)
            .clip(RoundedCornerShape(PetalTheme.metrics.form.controls.checkboxRadius))
            .background(bgColor)
            .border(
                PetalTheme.metrics.form.controls.checkboxBorderWidth,
                borderColor,
                RoundedCornerShape(PetalTheme.metrics.form.controls.checkboxRadius),
            ),
        contentAlignment = Alignment.Center,
    ) {
        when {
            isChecked -> MateIcon(
                name = "check",
                size = size - PetalTheme.metrics.form.controls.checkboxCheckInset,
                tint = PetalTheme.colors.checkboxMark,
            )
            isIndeterminate -> {
                // 半选：1.5px 高白条，宽 size-9，radius 1
                Box(
                    modifier = Modifier
                        .width(size - PetalTheme.metrics.form.controls.checkboxIndeterminateInset)
                        .height(PetalTheme.metrics.form.controls.checkboxIndeterminateHeight)
                        .clip(RoundedCornerShape(PetalTheme.metrics.form.controls.checkboxIndeterminateRadius))
                        .background(PetalTheme.colors.checkboxMark),
                )
            }
        }
    }
}

/**
 * 单选项（对标原 Vue `<MateRadio>`）。圆形，选中显示 brand 实心圆点。
 */
@Composable
fun MateRadio(
    selected: Boolean,
    onSelect: () -> Unit,
    modifier: Modifier = Modifier,
    size: Dp = PetalTheme.metrics.form.controls.radioDefaultSize,
    disabled: Boolean = false,
) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    val borderColor = if (selected) PetalTheme.colors.brand else semantic.border
    Box(
        modifier = modifier
            .then(
                if (disabled) Modifier
                else Modifier.clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onSelect),
            )
            .alpha(if (disabled) PetalTheme.metrics.form.controls.radioDisabledAlpha else 1f)
            .size(size)
            .clip(CircleShape)
            .background(semantic.bgContainer)
            .border(PetalTheme.metrics.form.controls.radioBorderWidth, borderColor, CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        if (selected) {
            // 实心圆点，直径 = size * 0.5
            Box(
                modifier = Modifier
                    .size(size * 0.5f)
                    .clip(CircleShape)
                    .background(PetalTheme.colors.brand),
            )
        }
    }
}
