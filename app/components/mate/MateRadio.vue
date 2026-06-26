<!-- 单选项，支持独立使用或配合 RadioGroup -->
<script setup lang="ts">
import { computed, inject } from "vue";

const props = withDefaults(defineProps<{
  value: string | number;
  /** 可选：脱离 group 独立用时直接传 */
  modelValue?: string | number | null;
  disabled?: boolean;
  size?: number;
}>(), { disabled: false, size: 16 });

const group = inject<{ value: string | number | null; select: (v: string | number) => void } | null>(
  "mate-radio-group",
  null,
);

const selected = computed(() => {
  const gv = group ? group.value : props.modelValue ?? null;
  return gv === props.value;
});

const sz = computed(() => `${props.size}px`);

const emit = defineEmits<{ (e: "update:modelValue", v: string | number): void }>();

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
  border: 1px solid var(--border);
  border-radius: 50%;
  background-color: var(--bg-container);
  cursor: pointer;
  padding: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  transition: border-color 0.12s;
}
.mate-radio:hover:not(.is-disabled) { border-color: var(--color-brand); }
.mate-radio.is-selected { border-color: var(--color-brand); }
.mate-radio.is-disabled { opacity: 0.5; cursor: not-allowed; }
.mate-radio__dot {
  border-radius: 50%;
  background-color: var(--color-brand);
}
</style>
