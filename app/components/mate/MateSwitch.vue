<!-- 开关（v2：iOS 风格胶囊，46×28，knob 22 白圆） -->
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
  width: 46px;
  height: 28px;
  border-radius: var(--radius-full);
  border: none;
  background-color: #e3e3e6;
  cursor: pointer;
  flex-shrink: 0;
  padding: 0;
  transition: background-color 0.18s;
}
.mate-switch.is-on { background-color: var(--brand-500); }
.mate-switch.is-disabled { opacity: 0.5; cursor: not-allowed; }
.mate-switch__knob {
  position: absolute;
  top: 3px;
  left: 3px;
  width: 22px;
  height: 22px;
  border-radius: 50%;
  background: #fff;
  box-shadow: 0 2px 5px rgba(0, 0, 0, 0.18);
  transition: transform 0.18s;
}
.mate-switch.is-on .mate-switch__knob { transform: translateX(18px); }
</style>
