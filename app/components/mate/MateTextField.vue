<!-- 文本输入框（v2：灰底无框，聚焦白底 + 品牌描边环），38px 高 -->
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
  height: 38px;
  width: 100%;
  padding: 0 var(--space-md);
  border: none;
  border-radius: var(--radius-md);
  background-color: var(--bg-fill);
  transition: box-shadow 0.12s, background-color 0.12s;
  gap: var(--space-sm);
}

/* v2 聚焦态：白底 + 品牌浅蓝描边环 */
.mate-text-field:focus-within {
  background-color: var(--bg-card);
  box-shadow: inset 0 0 0 2px var(--brand-200);
}

.mate-text-field.is-error {
  box-shadow: inset 0 0 0 2px var(--err);
}
.mate-text-field.is-error:focus-within {
  box-shadow: inset 0 0 0 2px var(--err);
}

.mate-text-field.is-disabled {
  opacity: 0.6;
}

.mate-text-field__prefix {
  color: var(--ink-300);
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
  color: var(--ink-900);
}

.mate-text-field__input::placeholder {
  color: var(--ink-300);
}
</style>
