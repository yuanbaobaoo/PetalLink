<!-- 单选组，provide/inject 向下传递选中值 -->
<script setup lang="ts">
import { provide, ref } from "vue";

// 组件输入参数。
const props = defineProps<{
  modelValue: string | number | null;
}>();

// 用 ref 持有当前值，provide 一个可被 MateRadio 调用的 select 函数
const current = ref<string | number | null>(props.modelValue);

// 组件事件发送器。
const emit = defineEmits<{ (e: "update:modelValue", v: string | number): void }>();
provide("mate-radio-group", {
  get value() { return current.value; },
  set value(v: string | number | null) { current.value = v; },
  select(v: string | number) {
    current.value = v;
    emit("update:modelValue", v);
  },
});
</script>

<template>
  <div class="mate-radio-group">
    <slot />
  </div>
</template>

<style scoped>
.mate-radio-group { display: inline-flex; flex-direction: column; gap: var(--space-sm); }
</style>
