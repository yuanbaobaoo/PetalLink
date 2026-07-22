<!-- 信息横幅，支持 info/success/warning/error 四种变体 -->
<script setup lang="ts">
import MateIcon from "./MateIcon.vue";

/** 横幅变体 */
type BannerVariant = "info" | "success" | "warning" | "error";

interface Props {
  variant?: BannerVariant;
  /** 标题 */
  title?: string;
  /** 是否可关闭 */
  closable?: boolean;
}

// info / success / warning / error 对应的默认图标
const defaultIcon: Record<BannerVariant, string> = {
  info: "info",
  success: "check",
  warning: "alert",
  error: "x",
};

const props = withDefaults(defineProps<Props>(), {
  variant: "info",
  title: "",
  closable: false,
});

const emit = defineEmits<{
  (e: "close"): void;
}>();

function handleClose(): void {
  emit("close");
}
</script>

<template>
  <div :class="['mate-banner', `mate-banner--${props.variant}`]">
    <MateIcon :name="defaultIcon[props.variant]" :size="18" class="mate-banner__icon" />
    <div class="mate-banner__content">
      <div v-if="title" class="mate-banner__title">{{ title }}</div>
      <div class="mate-banner__message">
        <slot />
      </div>
    </div>
    <!-- 操作按钮区（右侧） -->
    <div v-if="$slots.action" class="mate-banner__action">
      <slot name="action" />
    </div>
    <button
      v-if="closable"
      class="mate-banner__close"
      title="关闭"
      @click="handleClose"
    >
      <MateIcon name="x" :size="14" />
    </button>
  </div>
</template>

<style scoped>
.mate-banner {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 14px;
  border-radius: var(--radius-lg);
  font-size: var(--font-body-sm);
  line-height: 1.55;
  color: var(--ink-700);
}

.mate-banner--info {
  background-color: var(--brand-50);
}
.mate-banner--info .mate-banner__icon {
  color: var(--brand-500);
}
.mate-banner--success {
  background-color: var(--ok-bg);
}
.mate-banner--success .mate-banner__icon {
  color: var(--ok);
}
.mate-banner--warning {
  background-color: var(--warn-bg);
}
.mate-banner--warning .mate-banner__icon {
  color: var(--warn);
}
.mate-banner--error {
  background-color: var(--err-bg);
}
.mate-banner--error .mate-banner__icon {
  color: var(--err);
}

.mate-banner__icon {
  flex-shrink: 0;
  margin-top: 1px;
}

.mate-banner__content {
  flex: 1;
  min-width: 0;
}

.mate-banner__title {
  font-weight: var(--fw-semibold);
  margin-bottom: var(--space-xs);
  color: inherit;
}

.mate-banner__message {
  color: inherit;
  white-space: pre-line;
}

.mate-banner__action {
  margin-left: auto;
  flex-shrink: 0;
}

.mate-banner__close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: none;
  border: none;
  color: inherit;
  cursor: pointer;
  padding: 2px;
  line-height: 1;
  flex-shrink: 0;
  opacity: 0.7;
}
.mate-banner__close:hover {
  opacity: 1;
}
</style>
