<!-- 数值输入框，居中数字 + 可选单位后缀 -->
<script setup lang="ts">
interface Props {
  modelValue: number;
  min?: number;
  max?: number;
  width?: number;
  suffix?: string;
  disabled?: boolean;
}
const props = withDefaults(defineProps<Props>(), {
  min: 0,
  max: 999999,
  width: 120,
  suffix: "",
  disabled: false,
});
const emit = defineEmits<{ (e: "update:modelValue", v: number): void }>();

/**
 * 限制数值在 min/max 范围内
 *
 * @param v - 输入值
 */
function clamp(v: number): number {
  if (Number.isNaN(v)) return props.modelValue;
  return Math.max(props.min, Math.min(props.max, v));
}
/**
 * 处理输入事件
 *
 * @param event - 输入事件
 */
function handleInput(event: Event): void {
  const raw = (event.target as HTMLInputElement).value;
  const n = Number(raw);
  if (raw === "" || Number.isNaN(n)) return;
  emit("update:modelValue", clamp(n));
}
</script>

<template>
  <div class="mate-number-field" :style="{ width: `${width}px` }">
    <input
      type="number"
      class="mate-number-field__input"
      :value="modelValue"
      :min="min"
      :max="max"
      :disabled="disabled"
      @input="handleInput"
    />
    <span v-if="suffix" class="mate-number-field__suffix">{{ suffix }}</span>
  </div>
</template>

<style scoped>
.mate-number-field {
  display: inline-flex;
  align-items: center;
  gap: var(--space-sm);
  height: 32px;
  padding: 0 var(--space-md);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background-color: var(--bg-container);
  transition: border-color 0.15s;
}
.mate-number-field:focus-within { border-color: var(--color-brand); }
.mate-number-field__input {
  flex: 1;
  min-width: 0;
  height: 100%;
  border: none;
  outline: none;
  background: transparent;
  font-family: var(--font-family);
  font-size: var(--font-body);
  color: var(--text-primary);
  text-align: center;
}
/* 隐藏原生数字输入框的上下箭头 */
.mate-number-field__input::-webkit-outer-spin-button,
.mate-number-field__input::-webkit-inner-spin-button {
  -webkit-appearance: none;
  margin: 0;
}
.mate-number-field__suffix {
  font-size: var(--font-body-sm);
  color: var(--text-secondary);
  flex-shrink: 0;
}
</style>
