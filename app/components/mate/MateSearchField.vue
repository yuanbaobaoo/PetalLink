<!-- 搜索框，AppBar 用，prefix 搜索图标 -->
<script setup lang="ts">
import MateTextField from "./MateTextField.vue";

const props = withDefaults(defineProps<{
  modelValue?: string;
  placeholder?: string;
  width?: string;
  maxWidth?: number;
}>(), {
  modelValue: "",
  placeholder: "搜索文件和文件夹...",
  width: "",
  maxWidth: 0,
});

const emit = defineEmits<{
  (e: "update:modelValue", v: string): void;
  (e: "submit", v: string): void;
}>();

/**
 * 输入事件转发
 *
 * @param v - 输入值
 */
function onInput(v: string): void { emit("update:modelValue", v); }function onEnter(): void { emit("submit", ""); }
</script>

<template>
  <div class="mate-search-field" :style="props.maxWidth ? `maxWidth: ${props.maxWidth}px` : ''">
    <MateTextField
      :model-value="modelValue"
      :placeholder="placeholder"
      :width="width || '100%'"
      prefix-icon="search"
      font-size="font-body-sm"
      fill="var(--bg-page)"
      class="mate-search-field__input"
      @update:model-value="onInput"
      @enter="onEnter"
    />
  </div>
</template>

<style scoped>
.mate-search-field { display: inline-block; }
.mate-search-field :deep(.mate-text-field) {
  height: 38px;
}
</style>
