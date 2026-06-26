<script setup lang="ts">
/* 按钮组件，四种变体：primary / text / icon / icon-text。图标走 MateIcon，hover 仅背景色过渡。 */
import { computed, ref } from "vue";
import MateIcon from "./MateIcon.vue";

/** 按钮变体 */
export type ButtonVariant = "primary" | "text" | "icon" | "icon-text";

interface Props {
  variant?: ButtonVariant;
  /** 是否危险样式（红色） */
  danger?: boolean;
  /** 是否禁用 */
  disabled?: boolean;
  /** 是否加载中：primary 显示 spinner，其余变体图标转圈 + 禁用 */
  loading?: boolean;
  /** 是否全宽（仅 primary/text） */
  fullWidth?: boolean;
  /** tooltip 文本 */
  tooltip?: string;
  /** 图标名（icon-name，如 "cloud"/"x"/"refresh"，不带 i- 前缀） */
  icon?: string;
  /** 角标计数（>0 才显示，仅 icon / icon-text 变体；对齐 MateIconButtonWithText.badge） */
  badge?: number;
  /** 自定义高度（px） */
  height?: number;
}

// 悬停态（控制 hover 背景色，无 ripple）
const hovered = ref(false);

const props = withDefaults(defineProps<Props>(), {
  variant: "primary",
  danger: false,
  disabled: false,
  loading: false,
  fullWidth: false,
  tooltip: "",
  icon: "",
  badge: 0,
  height: 0,
});

// 按钮是否可交互
const clickable = computed(() => !props.disabled && !props.loading);

// primary 18 / text 14 / icon 18 / icon-text 16
const iconSize = computed(() => {
  switch (props.variant) {
    case "text":
      return 14;
    case "icon-text":
      return 16;
    default:
      return 18;
  }
});

// 是否显示角标（仅图标类变体）
const showBadge = computed(
  () => props.badge > 0 && (props.variant === "icon" || props.variant === "icon-text")
);

// 动态样式（避免在模板内用模板字符串，防止 SFC 解析器误判）
const heightStyle = computed(() =>
  props.height ? `height: ${props.height}px` : ""
);

const emit = defineEmits<{
  (e: "click", event: MouseEvent): void;
}>();

/**
 * 处理点击：禁用/加载中时不触发。
 */
function handleClick(event: MouseEvent): void {
  if (!clickable.value) return;
  emit("click", event);
}
</script>

<template>
  <button
    :class="[
      'mate-btn',
      `mate-btn--${variant}`,
      {
        'is-danger': danger,
        'is-disabled': disabled || loading,
        'is-full-width': fullWidth,
        'is-hover': hovered,
      },
    ]"
    :style="heightStyle"
    :title="tooltip"
    :disabled="disabled || loading"
    @click="handleClick"
    @mouseenter="hovered = true"
    @mouseleave="hovered = false"
  >
    <!-- 加载中 spinner（仅 primary） -->
    <span v-if="loading && variant === 'primary'" class="mate-btn__spinner" />
    <span v-else-if="icon" class="mate-btn__icon-wrap">
      <MateIcon :name="icon" :size="iconSize" :spin="loading" class="mate-btn__icon" />
      <span v-if="showBadge" class="mate-btn__badge">{{ badge > 99 ? "99+" : badge }}</span>
    </span>
    <span v-if="$slots.default" class="mate-btn__label"><slot /></span>
  </button>
</template>

<style scoped>
.mate-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-xs);
  font-family: var(--font-family);
  font-size: var(--font-body);
  font-weight: var(--fw-medium);
  border: none;
  cursor: pointer;
  transition: background-color 0.15s, color 0.15s;
  white-space: nowrap;
  flex-shrink: 0;
}

/* === primary：蓝色实心 === */
.mate-btn--primary {
  background-color: var(--color-brand);
  color: #fff;
  border-radius: var(--radius-sm);
  height: 36px;
  padding: 0 var(--space-lg);
}
.mate-btn--primary.is-hover {
  background-color: var(--color-brand-hover);
}
.mate-btn--primary:active {
  background-color: var(--color-brand-active);
}
.mate-btn--primary.is-full-width {
  width: 100%;
}
.mate-btn--primary.is-danger {
  background-color: var(--color-error);
}
.mate-btn--primary.is-disabled {
  background-color: var(--border);
  cursor: not-allowed;
}

/* === text：透明底蓝色文字 === */
.mate-btn--text {
  background-color: transparent;
  color: var(--color-brand);
  border-radius: var(--radius-sm);
  height: 36px;
  padding: 2px var(--space-sm);
  font-size: var(--font-body-sm);
}
.mate-btn--text.is-hover {
  background-color: var(--color-brand-lighter);
}
.mate-btn--text.is-danger {
  color: var(--color-error);
}

/* === icon：工具栏图标按钮（32×32） === */
.mate-btn--icon {
  background-color: transparent;
  color: var(--text-secondary);
  border-radius: var(--radius-sm);
  width: 32px;
  height: 32px;
}
.mate-btn--icon.is-hover {
  background-color: var(--bg-hover);
  color: var(--text-primary);
}
.mate-btn--icon.is-danger {
  color: var(--color-error);
}

/* === icon-text：带文字的图标按钮 === */
.mate-btn--icon-text {
  background-color: transparent;
  color: var(--text-primary);
  border-radius: var(--radius-sm);
  height: 32px;
  padding: 0 var(--space-md);
  font-size: var(--font-body-sm);
  gap: var(--space-xs);
}
.mate-btn--icon-text.is-hover {
  background-color: var(--bg-hover);
}
.mate-btn--icon-text.is-danger {
  color: var(--color-error);
}

/* spinner（加载中） */
.mate-btn__spinner {
  width: 16px;
  height: 16px;
  border: 2px solid rgba(255, 255, 255, 0.4);
  border-top-color: #fff;
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

.mate-btn__icon-wrap {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  line-height: 1;
}
.mate-btn__icon {
  display: inline-block;
}

/* 角标：18×18 全圆，brand 底白字 */
.mate-btn__badge {
  position: absolute;
  top: -6px;
  right: -8px;
  min-width: 18px;
  height: 18px;
  padding: 0 5px;
  border-radius: 9px;
  background-color: var(--color-brand);
  color: #fff;
  font-size: var(--font-caption);
  font-weight: var(--fw-semibold);
  line-height: 18px;
  text-align: center;
}
</style>
