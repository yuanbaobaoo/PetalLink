<!-- 文本输入框，自绘容器，32px 高 -->
<script setup lang="ts">
import { computed } from "vue";
import MateIcon from "./MateIcon.vue";

interface Props {
  modelValue?: string;
  placeholder?: string;
  /** 是否自动聚焦 */
  autofocus?: boolean;
  /** 是否禁用 */
  disabled?: boolean;
  /** 前缀图标 icon-name（如 "search"） */
  prefixIcon?: string;
  /** 自定义宽度（数字→px，字符串原样，如 "100%"/"280px"） */
  width?: number | string;
  /** 输入类型 */
  type?: string;
  /** 字号（默认 body 14；搜索框用 body-sm 13） */
  fontSize?: string;
  /** 填充色 token（默认 bg-container；搜索框用 bg-page） */
  fill?: string;
  /** 错误态（红色边框） */
  error?: boolean;
  /** 最大长度 */
  maxlength?: number;
}

interface Emits {
  (e: "update:modelValue", value: string): void;
  (e: "enter"): void;
  (e: "blur"): void;
}

const props = withDefaults(defineProps<Props>(), {
  modelValue: "",
  placeholder: "",
  autofocus: false,
  disabled: false,
  prefixIcon: "",
  width: "",
  type: "text",
  fontSize: "",
  fill: "",
  error: false,
  maxlength: 0,
});

// 宽度样式（支持数字 px 和字符串百分比）
const widthStyle = computed(() => {
  if (!props.width) return "";
  return typeof props.width === "number"
    ? `width: ${props.width}px`
    : `width: ${props.width}`;
});

// 填充色样式
const fillStyle = computed(() => (props.fill ? `background-color: ${props.fill}` : ""));
// 字体 CSS 变量
const fontCssVar = computed(() =>
  props.fontSize ? `var(--${props.fontSize})` : "var(--font-body)"
);

const emit = defineEmits<Emits>();

function handleInput(event: Event): void {
  const target = event.target as HTMLInputElement;
  emit("update:modelValue", target.value);
}

function handleEnter(): void {
  emit("enter");
}

function handleBlur(): void {
  emit("blur");
}
</script>

<template>
  <div
    class="mate-text-field"
    :class="{
      'is-disabled': disabled,
      'has-prefix': prefixIcon,
      'has-suffix': $slots.suffix,
      'is-error': error,
    }"
    :style="[widthStyle, fillStyle]"
  >
    <MateIcon v-if="prefixIcon" :name="prefixIcon" :size="16" class="mate-text-field__prefix" />
    <input
      :type="type"
      class="mate-text-field__input"
      :style="{ fontSize: fontCssVar }"
      :value="modelValue"
      :placeholder="placeholder"
      :disabled="disabled"
      :autofocus="autofocus"
      :maxlength="maxlength || undefined"
      @input="handleInput"
      @keyup.enter="handleEnter"
      @blur="handleBlur"
    />
    <slot name="suffix" />
  </div>
</template>

<style scoped>
.mate-text-field {
  display: inline-flex;
  align-items: center;
  height: 32px;
  width: 100%;
  padding: 0 var(--space-md);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background-color: var(--bg-container);
  transition: border-color 0.15s;
  gap: var(--space-sm);
}

.mate-text-field:focus-within {
  border-color: var(--color-brand);
}

.mate-text-field.is-error {
  border-color: var(--color-error);
}
.mate-text-field.is-error:focus-within {
  border-color: var(--color-error);
}

.mate-text-field.is-disabled {
  background-color: var(--bg-hover);
  opacity: 0.6;
}

.mate-text-field__prefix {
  color: var(--text-secondary);
  flex-shrink: 0;
}

.mate-text-field__input {
  flex: 1;
  min-width: 0;
  height: 100%;
  border: none;
  outline: none;
  background: transparent;
  font-family: var(--font-family);
  color: var(--text-primary);
}

.mate-text-field__input::placeholder {
  color: var(--text-placeholder);
}
</style>
