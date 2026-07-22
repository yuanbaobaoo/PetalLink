<!-- 复选框，支持 tri-state 半选态 -->
<script setup lang="ts">
import { computed } from "vue";
import MateIcon from "./MateIcon.vue";

const props = withDefaults(defineProps<{
  /** null = 半选（需 tristate） */
  modelValue: boolean | null;
  tristate?: boolean;
  disabled?: boolean;
  size?: number;
}>(), { tristate: false, disabled: false, size: 18 });

const checked = computed(() => props.modelValue === true);
const indeterminate = computed(() => props.tristate && props.modelValue === null);
const active = computed(() => checked.value || indeterminate.value);
const sz = computed(() => `${props.size}px`);

const emit = defineEmits<{ (e: "update:modelValue", v: boolean | null): void }>();

function toggle(): void {
  if (props.disabled) return;
  if (props.tristate) {
    // null → true → false → null
    if (props.modelValue === null) emit("update:modelValue", true);
    else if (props.modelValue === true) emit("update:modelValue", false);
    else emit("update:modelValue", null);
  } else {
    emit("update:modelValue", !(props.modelValue ?? false));
  }
}
</script>

<template>
  <button
    class="mate-checkbox"
    :class="{ 'is-active': active, 'is-disabled': props.disabled }"
    :style="{ width: sz, height: sz }"
    :disabled="props.disabled"
    @click="toggle"
  >
    <MateIcon v-if="checked" name="check" :size="props.size - 4" class="mate-checkbox__icon" />
    <span v-else-if="indeterminate" class="mate-checkbox__dash" :style="{ width: `${props.size - 8}px` }" />
  </button>
</template>

<style scoped>
.mate-checkbox {
  border: 1.5px solid var(--ink-300);
  border-radius: var(--radius-sm);
  background-color: var(--bg-card);
  cursor: pointer;
  padding: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  transition: background-color 0.12s, border-color 0.12s;
}
.mate-checkbox:hover:not(.is-disabled) { border-color: var(--brand-500); }
.mate-checkbox.is-active {
  background-color: var(--brand-500);
  border-color: var(--brand-500);
}
.mate-checkbox.is-disabled { opacity: 0.5; cursor: not-allowed; }
.mate-checkbox__icon { color: #fff; }
.mate-checkbox__dash {
  height: 1.5px;
  background-color: #fff;
  border-radius: 1px;
}
</style>
