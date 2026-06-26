<!--
  PetalLink 应用 Logo 组件 —— 圆角图标 + 可选文字 "PetalLink"。
  可复用于侧边栏、登录页、Splash 等场景。

  Props:
    size      图标高度 px（默认 26），文字自适配
    text      显示文字（默认 "PetalLink"，传空字符串隐藏）
    container 品牌色圆角容器包裹（登录卡片用，64×64 圆角 16 + brand 阴影）
-->
<script setup lang="ts">
import logoUrl from "@assets/logo.png";

const props = withDefaults(defineProps<{
  size?: number;
  text?: string;
  container?: boolean;
}>(), {
  size: 26,
  text: "PetalLink",
  container: false,
});

const emit = defineEmits<{ (e: "error"): void }>();

function onImgError(): void {
  emit("error");
}
</script>

<template>
  <div
    v-if="props.container"
    class="app-logo app-logo--container"
  >
		    <img
		      :src="logoUrl"
		      alt="PetalLink"
		      class="app-logo__img app-logo__img--container"
		      @error="onImgError"
	      />
  </div>
  <div
    v-else
    class="app-logo"
    :style="{ height: `${props.size}px` }"
  >
		    <img
		      :src="logoUrl"
		      alt="PetalLink"
		      class="app-logo__img"
		      :style="{ height: `${props.size}px`, width: `${props.size}px` }"
		      @error="onImgError"
		    />
    <p
      v-if="props.text"
      class="app-logo__text"
      :style="{ fontSize: `${Math.round(props.size * 0.42)}px` }"
    >{{ props.text }}</p>
  </div>
</template>

<style scoped>
.app-logo {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}
.app-logo--container {
  justify-content: center;
  padding: 5px;
  border-radius: 16px;
  box-shadow: 0 2px 8px rgba(0, 82, 217, 0.16);
}
.app-logo__img {
  border-radius: 6px;
  object-fit: contain;
}
.app-logo__img--container {
  width: 64px;
  height: 64px;
  border-radius: 0;
}
.app-logo__text {
  margin: 0;
  font-weight: var(--fw-semibold);
  color: #181818;
  line-height: 1;
}
</style>
