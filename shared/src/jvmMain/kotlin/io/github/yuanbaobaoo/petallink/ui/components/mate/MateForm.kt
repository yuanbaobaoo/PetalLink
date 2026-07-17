@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.components.mate

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.ErrorColor
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors

/**
 * 自绘容器文本输入（对标原 Vue `<MateTextField>`）。
 *
 * h32，1px border，radius-sm，bg-container；focus border→brand；error border→error；
 * disabled bg-hover opacity 0.6；prefix 图标 text-secondary；placeholder text-placeholder。
 *
 * @param value 当前文本
 * @param onValueChange 文本变化回调
 * @param placeholder 占位提示
 * @param modifier 外部 Modifier
 * @param enabled 是否启用
 * @param prefixIcon 前缀图标 name（可选）
 * @param error 错误态（红色边框）
 * @param singleLine 单行（默认 true）
 * @param fontSize 字号 sp（默认 14）
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
    fontSize: Int = 14,
    keyboardOptions: KeyboardOptions = KeyboardOptions.Default,
    visualTransformation: VisualTransformation = VisualTransformation.None,
    suffix: @Composable (() -> Unit)? = null,
) {
    val interaction = remember { MutableInteractionSource() }
    val focused by interaction.collectIsFocusedAsState()
    val semantic = LocalSemanticColors.current
    val borderColor = when {
        !enabled -> semantic.border
        error -> ErrorColor
        focused -> BrandColor
        else -> semantic.border
    }
    Row(
        modifier = modifier
            .height(32.dp)
            .clip(RoundedCornerShape(3.dp))
            .background(if (enabled) semantic.bgContainer else semantic.bgHover)
            .border(width = 1.dp, color = borderColor, shape = RoundedCornerShape(3.dp))
            .alpha(if (enabled) 1f else 0.6f)
            .padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (prefixIcon != null) {
            MateIcon(name = prefixIcon, size = 16.dp, tint = semantic.textSecondary)
        }
        Box(modifier = Modifier.weight(1f)) {
            // BasicTextField 无 ripple，完全自绘光标
            BasicTextField(
                value = value,
                onValueChange = onValueChange,
                modifier = Modifier.fillMaxWidth(),
                enabled = enabled,
                singleLine = singleLine,
                textStyle = TextStyle(
                    color = semantic.textPrimary,
                    fontSize = fontSize.sp,
                ),
                cursorBrush = SolidColor(BrandColor),
                interactionSource = interaction,
                keyboardOptions = keyboardOptions,
                visualTransformation = visualTransformation,
            )
            if (value.isEmpty() && placeholder.isNotEmpty()) {
                Text(
                    placeholder,
                    color = semantic.textPlaceholder,
                    fontSize = fontSize.sp,
                )
            }
        }
        suffix?.invoke()
    }
}

/**
 * 数值输入（对标原 Vue `<MateNumberField>`）。
 *
 * 居中数字 + 可选单位后缀；min/max clamp；h32，1px border，radius-sm。
 * 隐藏原生 spin button（BasicTextField 无 spin）。
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
    val semantic = LocalSemanticColors.current
    val interaction = remember { MutableInteractionSource() }
    val focused by interaction.collectIsFocusedAsState()
    val borderColor = if (!enabled) semantic.border else if (focused) BrandColor else semantic.border

    Row(
        modifier = modifier
            .height(32.dp)
            .clip(RoundedCornerShape(3.dp))
            .background(if (enabled) semantic.bgContainer else semantic.bgHover)
            .border(1.dp, borderColor, RoundedCornerShape(3.dp))
            .padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Box(modifier = Modifier.weight(1f)) {
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
                textStyle = TextStyle(
                    color = semantic.textPrimary,
                    fontSize = 14.sp,
                    textAlign = TextAlign.Center,
                ),
                cursorBrush = SolidColor(BrandColor),
                interactionSource = interaction,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
            )
        }
        if (suffix.isNotEmpty()) {
            Text(suffix, color = semantic.textSecondary, fontSize = 13.sp)
        }
    }
}

/**
 * 步进器（对标原 Vue `<MateStepper>`）。
 *
 * [−|值|+]，h32，1px border，radius-sm，overflow hidden；
 * 按钮 32×32 透明，hover bg-hover；disabled text-placeholder；
 * minus 用 x 图标 rotate 45° 变减号，plus 字号 18px；中间 value 宽 48 居中 medium。
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
    val semantic = LocalSemanticColors.current
    val minusEnabled = value > min
    val plusEnabled = value < max

    Row(
        modifier = modifier
            .height(32.dp)
            .clip(RoundedCornerShape(3.dp))
            .background(semantic.bgContainer)
            .border(1.dp, semantic.border, RoundedCornerShape(3.dp)),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // 减号按钮（x 图标 rotate 45° 近似减号）
        Box(
            modifier = Modifier
                .size(32.dp)
                .alpha(if (minusEnabled) 1f else 0.4f)
                .then(
                    if (minusEnabled) Modifier.background(semantic.bgHover.copy(alpha = 0f))
                        .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {
                            onValueChange((value - step).coerceIn(min, max))
                        }
                    else Modifier,
                ),
            contentAlignment = Alignment.Center,
        ) {
            // 用 x 旋转 45 度作为减号（与原 Vue 一致）
            MateIcon(name = "x", size = 14.dp, tint = semantic.textSecondary, modifier = Modifier.scale(0.8f).rotate45())
        }
        // 中间数值（宽 48，居中，medium）
        Box(
            modifier = Modifier.width(48.dp),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                value.toString(),
                color = semantic.textPrimary,
                fontSize = 14.sp,
                fontWeight = FontWeight.Medium,
            )
        }
        // 加号按钮
        Box(
            modifier = Modifier
                .size(32.dp)
                .alpha(if (plusEnabled) 1f else 0.4f)
                .then(
                    if (plusEnabled) Modifier.clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {
                        onValueChange((value + step).coerceIn(min, max))
                    } else Modifier,
                ),
            contentAlignment = Alignment.Center,
        ) {
            Text("+", color = semantic.textSecondary, fontSize = 18.sp, fontWeight = FontWeight.Medium)
        }
    }
}

/** x 图标旋转 45° 近似减号的 Modifier（封装保持调用点简洁）。 */
private fun Modifier.rotate45(): Modifier = this.scale(scaleX = 0.8f, scaleY = 0.8f).rotate(45f)

/**
 * 搜索框（对标原 Vue `<MateSearchField>`）。
 *
 * 内嵌 [MateTextField] + search 前缀图标；h30；bg=bg-page（与 AppBar 区分）。
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
    maxWidth: Dp = 0.dp,
    onSubmit: (String) -> Unit = {},
) {
    val widthMod = if (maxWidth > 0.dp) Modifier.width(maxWidth) else Modifier.fillMaxWidth()
    val semantic = LocalSemanticColors.current
    Box(modifier = modifier.then(widthMod).height(30.dp)) {
        MateTextField(
            value = value,
            onValueChange = onValueChange,
            placeholder = placeholder,
            modifier = Modifier.fillMaxWidth(),
            prefixIcon = "search",
            fontSize = 13,
            keyboardOptions = KeyboardOptions.Default.copy(
                imeAction = androidx.compose.ui.text.input.ImeAction.Search,
            ),
        )
    }
}

/**
 * 开关（对标原 Vue `<MateSwitch>`）。
 *
 * 40×22，radius 11，bg=border(off)/brand(on)，过渡 0.2s；
 * knob 18×18 白圆 top/left 2，box-shadow，on 时 translateX 18；disabled opacity 0.5。
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
    val semantic = LocalSemanticColors.current
    val trackColor = if (checked) BrandColor else semantic.border
    val knobOffset = if (checked) 18.dp else 2.dp
    Row(
        modifier = modifier
            .then(
                if (disabled) Modifier
                else Modifier.clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {
                    onCheckedChange(!checked)
                },
            )
            .alpha(if (disabled) 0.5f else 1f)
            .size(40.dp, 22.dp)
            .clip(RoundedCornerShape(11.dp))
            .background(trackColor),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // knob 18×18 白圆，靠左偏移 2(off) 或 18(on)
        Box(
            modifier = Modifier
                .padding(start = knobOffset)
                .size(18.dp)
                .clip(CircleShape)
                .background(Color.White),
        )
    }
}

/**
 * 复选框（对标原 Vue `<MateCheckbox>`，支持 tri-state）。
 *
 * 1px border，radius-sm，bg-container；hover border→brand；
 * active bg=brand border=brand，显示白色 check 图标；
 * 半选（null）显示 1.5px 高白条；disabled opacity 0.5。
 *
 * @param checked 三态：true/false/null(半选)
 * @param onCheckedChange 状态变化（tri-state 循环 null→true→false→null）
 * @param size 尺寸（默认 16）
 * @param disabled 是否禁用
 */
@Composable
fun MateCheckbox(
    checked: Boolean?,
    onCheckedChange: (Boolean?) -> Unit,
    modifier: Modifier = Modifier,
    size: Dp = 16.dp,
    disabled: Boolean = false,
) {
    val semantic = LocalSemanticColors.current
    val interaction = remember { MutableInteractionSource() }
    val hovered by interaction.collectIsFocusedAsState()
    val isChecked = checked == true
    val isIndeterminate = checked == null
    val borderColor = when {
        isChecked || isIndeterminate -> BrandColor
        hovered && !disabled -> BrandColor
        else -> semantic.border
    }
    val bgColor = if (isChecked || isIndeterminate) BrandColor else semantic.bgContainer

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
            .alpha(if (disabled) 0.5f else 1f)
            .size(size)
            .clip(RoundedCornerShape(3.dp))
            .background(bgColor)
            .border(1.dp, borderColor, RoundedCornerShape(3.dp)),
        contentAlignment = Alignment.Center,
    ) {
        when {
            isChecked -> MateIcon(name = "check", size = (size - 4.dp), tint = Color.White)
            isIndeterminate -> {
                // 半选：1.5px 高白条，宽 size-8，radius 1
                Box(
                    modifier = Modifier
                        .width(size - 8.dp)
                        .height(1.5.dp)
                        .clip(RoundedCornerShape(1.dp))
                        .background(Color.White),
                )
            }
        }
    }
}

/** 单选项（对标原 Vue `<MateRadio>`）。圆形，选中显示 brand 实心圆点。 */
@Composable
fun MateRadio(
    selected: Boolean,
    onSelect: () -> Unit,
    modifier: Modifier = Modifier,
    size: Dp = 16.dp,
    disabled: Boolean = false,
) {
    val semantic = LocalSemanticColors.current
    val borderColor = if (selected) BrandColor else semantic.border
    Box(
        modifier = modifier
            .then(
                if (disabled) Modifier
                else Modifier.clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onSelect),
            )
            .alpha(if (disabled) 0.5f else 1f)
            .size(size)
            .clip(CircleShape)
            .background(semantic.bgContainer)
            .border(1.dp, borderColor, CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        if (selected) {
            // 实心圆点，直径 = size * 0.5
            Box(
                modifier = Modifier
                    .size(size * 0.5f)
                    .clip(CircleShape)
                    .background(BrandColor),
            )
        }
    }
}
