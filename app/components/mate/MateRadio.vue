<!-- 单选项，支持独立使用或配合 RadioGroup -->
<script setup lang="ts">
import { computed, inject } from "vue";

// 组件输入参数。
const props = withDefaults(defineProps<{
  value: string | number;
  /**
   * 可选：脱离 group 独立用时直接传
   */
  modelValue?: string | number | null;
  disabled?: boolean;
  size?: number;
}>(), { disabled: false, size: 16 });

// 当前注入的单选组上下文。
const group = inject<{ value: string | number | null; select: (v: string | number) => void } | null>(
  "mate-radio-group",
  null,
);

// 当前选项是否选中或用户选择结果。
const selected = computed(() => {
  // 单选组提供的当前值。
  const gv = group ? group.value : props.modelValue ?? null;
  return gv === props.value;
});

// 组件尺寸对应的 CSS 值。
const sz = computed(() => `${props.size}px`);

// 组件事件发送器。
const emit = defineEmits<{ (e: "update:modelValue", v: string | number): void }>();

/**
 * 选择当前单选项并提交新值。
 */
function choose(): void {
  if (props.disabled) return;
  if (group) group.select(props.value);
  else emit("update:modelValue", props.value);
}
</script>

<template>
  <button
    class="mate-radio"
    :class="{ 'is-selected': selected, 'is-disabled': props.disabled }"
    :style="{ width: sz, height: sz }"
    :disabled="props.disabled"
    @click="choose"
  >
    <span v-if="selected" class="mate-radio__dot" :style="{ width: `${props.size * 0.5}px`, height: `${props.size * 0.5}px` }" />
  </button>
</template>

<style scoped>
.mate-radio {
  border: 1.5px solid var(--ink-300);
  border-radius: 50%;
  background-color: var(--bg-card);
  cursor: pointer;
  padding: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  transition: border-color 0.12s;
}
.mate-radio:hover:not(.is-disabled) { border-color: var(--brand-500); }
.mate-radio.is-selected { border-color: var(--brand-500); }
.mate-radio.is-disabled { opacity: 0.5; cursor: not-allowed; }
.mate-radio__dot {
  border-radius: 50%;
  background-color: var(--brand-500);
}
</style>
