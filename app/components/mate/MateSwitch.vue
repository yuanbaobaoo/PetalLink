<!-- 开关，40×22，knob 18 白圆 -->
<script setup lang="ts">
const props = withDefaults(defineProps<{
  modelValue: boolean;
  disabled?: boolean;
}>(), { disabled: false });
const emit = defineEmits<{ (e: "update:modelValue", v: boolean): void }>();

function toggle(): void { emit("update:modelValue", !props.modelValue); }
</script>

<template>
  <button
    class="mate-switch"
    :class="{ 'is-on': modelValue, 'is-disabled': disabled }"
    :disabled="disabled"
    @click="toggle"
  >
    <span class="mate-switch__knob" />
  </button>
</template>

<style scoped>
.mate-switch {
  position: relative;
  width: 40px;
  height: 22px;
  border-radius: 11px;
  border: none;
  background-color: var(--border);
  cursor: pointer;
  flex-shrink: 0;
  padding: 0;
  transition: background-color 0.2s ease-in-out;
}
.mate-switch.is-on { background-color: var(--color-brand); }
.mate-switch.is-disabled { opacity: 0.5; cursor: not-allowed; }
.mate-switch__knob {
  position: absolute;
  top: 2px;
  left: 2px;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: #fff;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
  transition: transform 0.2s ease-in-out;
}
.mate-switch.is-on .mate-switch__knob { transform: translateX(18px); }
</style>
