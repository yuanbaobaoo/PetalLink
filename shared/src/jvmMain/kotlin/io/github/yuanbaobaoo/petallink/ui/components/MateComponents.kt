package io.github.yuanbaobaoo.petallink.ui.components

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

// === MateButton ===
@Composable
fun MateButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    primary: Boolean = true,
) {
    Button(
        onClick = onClick,
        enabled = enabled,
        modifier = modifier,
        colors = if (primary) ButtonDefaults.buttonColors() else ButtonDefaults.outlinedButtonColors(),
    ) { Text(text) }
}

// === MateTextField ===
@Composable
fun MateTextField(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        label = { Text(label) },
        modifier = modifier.fillMaxWidth(),
        enabled = enabled,
        singleLine = true,
    )
}

// === MateTag ===
@Composable
fun MateTag(text: String, color: Color = MaterialTheme.colors.primary) {
    Surface(color = color.copy(alpha = 0.1f), shape = RoundedCornerShape(4.dp)) {
        Text(text, color = color, fontSize = 12.sp, modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp))
    }
}

// === MateSectionHeader ===
@Composable
fun MateSectionHeader(title: String, subtitle: String? = null) {
    Column {
        Text(title, fontSize = 16.sp, fontWeight = FontWeight.Bold)
        subtitle?.let { Text(it, fontSize = 12.sp, color = Color.Gray) }
    }
}

// === MateFileItem ===
data class MateFileItemData(
    val name: String,
    val size: String,
    val modified: String,
    val isFolder: Boolean,
    val statusIcon: String,
)

@Composable
fun MateFileItem(item: MateFileItemData, onClick: () -> Unit = {}) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(if (item.isFolder) "📁" else "📄", fontSize = 16.sp)
        Spacer(Modifier.width(8.dp))
        Text(item.name, modifier = Modifier.weight(1f), fontSize = 13.sp)
        Text(item.size, fontSize = 12.sp, color = Color.Gray, modifier = Modifier.width(80.dp))
        Text(item.modified, fontSize = 12.sp, color = Color.Gray, modifier = Modifier.width(120.dp))
        Text(item.statusIcon, fontSize = 14.sp)
    }
}

// === MateProgress ===
@Composable
fun MateProgress(percent: Float, modifier: Modifier = Modifier) {
    LinearProgressIndicator(progress = percent, modifier = modifier.fillMaxWidth().height(4.dp))
}

// === MateTransferItem ===
data class MateTransferItemData(
    val fileName: String,
    val direction: String,
    val progress: Float,
    val stateText: String,
    val id: Long? = null,
    val errorMessage: String? = null,
)

@Composable
fun MateTransferItem(item: MateTransferItemData) {
    Column(modifier = Modifier.fillMaxWidth().padding(8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(if (item.direction == "upload") "⬆️" else "⬇️", fontSize = 14.sp)
            Spacer(Modifier.width(8.dp))
            Text(item.fileName, fontSize = 13.sp, modifier = Modifier.weight(1f))
            Text(item.stateText, fontSize = 12.sp, color = Color.Gray)
        }
        Spacer(Modifier.height(4.dp))
        MateProgress(item.progress)
    }
}

// === MateEmpty ===
@Composable
fun MateEmpty(icon: String, title: String, description: String? = null) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(icon, fontSize = 48.sp)
        Spacer(Modifier.height(8.dp))
        Text(title, fontSize = 14.sp, fontWeight = FontWeight.Medium)
        description?.let {
            Spacer(Modifier.height(4.dp))
            Text(it, fontSize = 12.sp, color = Color.Gray)
        }
    }
}

// === MateSwitch ===
@Composable
fun MateSwitch(checked: Boolean, onCheckedChange: (Boolean) -> Unit, label: String? = null) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        label?.let {
            Text(it, fontSize = 14.sp)
            Spacer(Modifier.width(8.dp))
        }
        Switch(checked = checked, onCheckedChange = onCheckedChange)
    }
}
