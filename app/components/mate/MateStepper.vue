<!-- 步进器 [− | 值 | +] -->
<script setup lang="ts">
import { computed } from "vue";
import MateIcon from "./MateIcon.vue";
import MateVerticalSeparator from "./MateVerticalSeparator.vue";

interface Props {
  modelValue: number;
  min?: number;
  max?: number;
  step?: number;
}
const props = withDefaults(defineProps<Props>(), { min: 0, max: 999999, step: 1 });

const canDec = computed(() => props.modelValue > props.min);
const canInc = computed(() => props.modelValue < props.max);

const emit = defineEmits<{ (e: "update:modelValue", v: number): void }>();

/**
 * 减少步长
 */
function dec(): void {
  if (canDec.value) emit("update:modelValue", Math.max(props.min, props.modelValue - props.step));
}
/**
 * 增加步长
 */
function inc(): void {
  if (canInc.value) emit("update:modelValue", Math.min(props.max, props.modelValue + props.step));
}
</script>

<template>
  <div class="mate-stepper">
    <button class="mate-stepper__btn" :class="{ 'is-disabled': !canDec }" :disabled="!canDec" @click="dec">
      <MateIcon name="x" :size="16" class="mate-stepper__minus" />
    </button>
    <MateVerticalSeparator :height="20" />
    <span class="mate-stepper__val">{{ modelValue }}</span>
    <MateVerticalSeparator :height="20" />
    <button class="mate-stepper__btn" :class="{ 'is-disabled': !canInc }" :disabled="!canInc" @click="inc">
      <span class="mate-stepper__plus">+</span>
    </button>
  </div>
</template>

<style scoped>
.mate-stepper {
  display: inline-flex;
  align-items: center;
  height: 32px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background-color: var(--bg-container);
  overflow: hidden;
}
.mate-stepper__btn {
  width: 32px;
  height: 100%;
  border: none;
  background: transparent;
  cursor: pointer;
  color: var(--text-secondary);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  transition: background-color 0.15s;
}
.mate-stepper__btn:hover:not(.is-disabled) { background-color: var(--bg-hover); color: var(--text-primary); }
.mate-stepper__btn.is-disabled { color: var(--text-placeholder); cursor: not-allowed; }
/* 把 x 图标旋转成减号 */
.mate-stepper__minus { transform: rotate(45deg); }
.mate-stepper__plus { font-size: 18px; line-height: 1; }
.mate-stepper__val {
  width: 48px;
  text-align: center;
  font-size: var(--font-body);
  font-weight: var(--fw-medium);
  color: var(--text-primary);
}
</style>
