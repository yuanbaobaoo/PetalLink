<!-- 对话框宿主，绑定 useDialog 模块状态 -->
<script setup lang="ts">
import MateDialog from "./MateDialog.vue";
import MateButton from "./MateButton.vue";
import { dialogState, closeDialog } from "./useDialog";

/**
 * 对话框开闭变化处理
 *
 * @param v - 是否打开
 */
function onOpenChange(v: boolean): void {
  // 关闭（遮罩/esc）→ 视为取消
  if (!v) closeDialog(false);
}
</script>

<template>
  <MateDialog
    :open="dialogState.open"
    :title="dialogState.title"
    :title-icon="dialogState.titleIcon"
    :danger="dialogState.danger"
    :close-on-overlay="dialogState.closeOnOverlay"
    :width="dialogState.width"
    @update:open="onOpenChange"
  >
    <div style="white-space: pre-line">{{ dialogState.content }}</div>
    <template v-if="dialogState.isConfirm" #footer>
      <MateButton variant="text" @click="closeDialog(false)">{{ dialogState.cancelText }}</MateButton>
      <MateButton variant="primary" :danger="dialogState.danger" @click="closeDialog(true)">
        {{ dialogState.confirmText }}
      </MateButton>
    </template>
  </MateDialog>
</template>
