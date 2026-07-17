package io.github.yuanbaobaoo.petallink.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import io.github.yuanbaobaoo.petallink.ui.theme.*

// ========== 对话框 ==========
@Composable
fun MateDialog(
    visible: Boolean,
    title: String,
    content: String,
    confirmText: String = "确定",
    cancelText: String = "取消",
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    if (visible) {
        Dialog(onDismissRequest = onDismiss) {
            Card(shape = RoundedCornerShape(12.dp)) {
                Column(modifier = Modifier.padding(24.dp)) {
                    Text(title, fontSize = 16.sp, fontWeight = FontWeight.Bold)
                    Spacer(Modifier.height(12.dp))
                    Text(content, fontSize = 13.sp, color = Color.Gray)
                    Spacer(Modifier.height(20.dp))
                    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                        TextButton(onClick = onDismiss) { Text(cancelText) }
                        Spacer(Modifier.width(8.dp))
                        Button(onClick = onConfirm) { Text(confirmText) }
                    }
                }
            }
        }
    }
}

// ========== Toast ==========
@Composable
fun MateToast(message: String, visible: Boolean, onDismiss: () -> Unit) {
    if (visible) {
        LaunchedEffect(message) {
            kotlinx.coroutines.delay(3000)
            onDismiss()
        }
        Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.BottomCenter) {
            Surface(
                modifier = Modifier.padding(32.dp),
                shape = RoundedCornerShape(8.dp),
                color = Color(0xDD333333),
            ) {
                Text(message, color = Color.White, fontSize = 14.sp, modifier = Modifier.padding(horizontal = 20.dp, vertical = 12.dp))
            }
        }
    }
}

// ========== 弹出菜单 ==========
@Composable
fun MatePopupMenu(
    items: List<Pair<String, () -> Unit>>,
    onDismiss: () -> Unit,
) {
    Card(elevation = 8.dp, shape = RoundedCornerShape(8.dp)) {
        Column(modifier = Modifier.width(180.dp)) {
            items.forEach { (text, onClick) ->
                Text(
                    text,
                    modifier = Modifier.clickable { onClick(); onDismiss() }.padding(12.dp).fillMaxWidth(),
                    fontSize = 13.sp,
                )
            }
        }
    }
}

// ========== 复选框 ==========
@Composable
fun MateCheckbox(checked: Boolean, onCheckedChange: (Boolean) -> Unit, label: String? = null, indeterminate: Boolean = false) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Checkbox(
            checked = if (indeterminate) false else checked,
            onCheckedChange = onCheckedChange,
        )
        label?.let {
            Spacer(Modifier.width(4.dp))
            Text(it, fontSize = 14.sp)
        }
    }
}

// ========== 单选组 ==========
@Composable
fun MateRadioGroup(options: List<String>, selected: Int, onSelect: (Int) -> Unit) {
    Column {
        options.forEachIndexed { idx, label ->
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.clickable { onSelect(idx) }) {
                RadioButton(selected = selected == idx, onClick = { onSelect(idx) })
                Spacer(Modifier.width(4.dp))
                Text(label, fontSize = 14.sp)
            }
        }
    }
}

// ========== 步进器 ==========
@Composable
fun MateStepper(value: Int, onValueChange: (Int) -> Unit, min: Int = 1, max: Int = 20) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        IconButton(onClick = { if (value > min) onValueChange(value - 1) }) { Text("-", fontSize = 18.sp) }
        Text(value.toString(), fontSize = 14.sp, modifier = Modifier.padding(horizontal = 12.dp))
        IconButton(onClick = { if (value < max) onValueChange(value + 1) }) { Text("+", fontSize = 18.sp) }
    }
}

// ========== 图标 ==========
@Composable
fun MateIcon(emoji: String, size: Dp = 24.dp, onClick: (() -> Unit)? = null) {
    val modifier = if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier
    Text(emoji, fontSize = size.value.sp, modifier = modifier)
}

// ========== 工具栏按钮 ==========
@Composable
fun MateIconButton(emoji: String, tooltip: String, onClick: () -> Unit) {
    Text(emoji, fontSize = 18.sp, modifier = Modifier.clickable(onClick = onClick).padding(4.dp))
}

// ========== 搜索框 ==========
@Composable
fun MateSearchField(value: String, onValueChange: (String) -> Unit, placeholder: String = "搜索...") {
    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        placeholder = { Text(placeholder) },
        modifier = Modifier.fillMaxWidth().height(48.dp),
        leadingIcon = { Text("🔍", fontSize = 14.sp) },
        singleLine = true,
    )
}

// ========== 数字输入 ==========
@Composable
fun MateNumberField(value: String, onValueChange: (String) -> Unit, label: String, min: Int = 1, max: Int = Int.MAX_VALUE) {
    OutlinedTextField(
        value = value,
        onValueChange = { v -> if (v.isEmpty() || v.toIntOrNull() in min..max) onValueChange(v) },
        label = { Text(label) },
        singleLine = true,
    )
}

// ========== 统计芯片 ==========
@Composable
fun MateStatChip(count: Int, label: String) {
    Surface(shape = RoundedCornerShape(12.dp), color = BrandColor.copy(alpha = 0.1f)) {
        Row(modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(count.toString(), fontSize = 14.sp, fontWeight = FontWeight.Bold, color = BrandColor)
            Spacer(Modifier.width(4.dp))
            Text(label, fontSize = 11.sp, color = BrandColor)
        }
    }
}

// ========== 加载动画 ==========
@Composable
fun MateSpinner(size: Dp = 24.dp) {
    MateCircularProgress(size)
}

// ========== 徽章 ==========
@Composable
fun MateBadge(count: Int, maxDisplay: Int = 99, color: Color = ErrorColor) {
    if (count > 0) {
        Box(
            modifier = Modifier.clip(CircleShape).background(color).size(18.dp),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                if (count > maxDisplay) "$maxDisplay+" else count.toString(),
                color = Color.White,
                fontSize = 10.sp,
            )
        }
    }
}

// ========== 信息横幅 ==========
@Composable
fun MateInfoBanner(text: String, type: BannerType = BannerType.INFO) {
    val bg = when (type) {
        BannerType.INFO -> Color(0xFFE8F4FD)
        BannerType.WARNING -> Color(0xFFFFF3E0)
        BannerType.ERROR -> Color(0xFFFFEBEE)
    }
    Surface(color = bg, shape = RoundedCornerShape(8.dp)) {
        Text(text, fontSize = 13.sp, modifier = Modifier.padding(12.dp).fillMaxWidth())
    }
}

enum class BannerType { INFO, WARNING, ERROR }
