<!-- 弹出菜单，触发器 + teleport，边界自动翻转 -->
<script setup lang="ts">
import { ref } from "vue";
import MateIcon from "./MateIcon.vue";

// 组件渲染 fragment（span + Teleport），Vue 无法自动继承 fallthrough attrs
defineOptions({ inheritAttrs: false });

export interface PopupItem {
  value: string | number;
  label?: string;
  icon?: string;
  danger?: boolean;
  /** true = 只画分隔线 */
  divider?: boolean;
}

const open = ref(false);
const top = ref(0);
const left = ref(0);
const triggerEl = ref<HTMLElement | null>(null);

const props = withDefaults(defineProps<{
  items: PopupItem[];
  menuWidth?: number;
  disabled?: boolean;
}>(), { menuWidth: 168, disabled: false });

const emit = defineEmits<{ (e: "select", value: string | number): void }>();

/**
 * 打开弹出菜单（pointerdown 触发）
 *
 * @param ev - 指针事件
 */
function openMenu(ev: PointerEvent): void {
  if (props.disabled) return;
  // 阻止本帧立即被捕获层关掉
  ev.stopPropagation();
  const rect = triggerEl.value?.getBoundingClientRect();
  if (!rect) return;
  const margin = 8;
  const menuH = 200; // 预估高度，用于上下翻转
  // 默认：菜单左对齐触发器左边缘（避免右截断）
  let nextLeft = rect.left;
  let nextTop = rect.bottom + 2;
  // 右边界检查：超出则右对齐视口右边缘
  if (nextLeft + props.menuWidth > window.innerWidth - margin) {
    nextLeft = window.innerWidth - margin - props.menuWidth;
  }
  // 左边界检查
  if (nextLeft < margin) nextLeft = margin;
  // 下边界检查：超出则向上弹出
  if (nextTop + menuH > window.innerHeight - margin) {
    nextTop = Math.max(margin, rect.top - menuH - 2);
  }
  left.value = nextLeft;
  top.value = nextTop;
  open.value = true;
}

function close(): void {
  open.value = false;
}

/**
 * 选中菜单项
 *
 * @param item - 选中的菜单项
 */
function select(item: PopupItem): void {
  if (item.divider) return;
  close();
  emit("select", item.value);
}
</script>

<template>
    <span ref="triggerEl" class="mate-popup-trigger" v-bind="$attrs" @pointerdown="openMenu">
    <slot />
  </span>
  <Teleport to="body">
    <!-- 全屏捕获层：点击/右键关闭 -->
    <div v-if="open" class="mate-popup-capture" @click="close" @contextmenu.prevent="close" />
    <div
      v-if="open"
      class="mate-popup-menu menu-fade-in"
      :style="{ top: `${top}px`, left: `${left}px`, width: `${menuWidth}px` }"
    >
      <template v-for="(item, i) in items" :key="i">
        <div v-if="item.divider" class="mate-popup-menu__divider" />
        <button
          v-else
          class="mate-popup-menu__item"
          :class="{ 'is-danger': item.danger }"
          @click="select(item)"
        >
          <MateIcon v-if="item.icon" :name="item.icon" :size="16" />
          <span>{{ item.label }}</span>
        </button>
      </template>
    </div>
  </Teleport>
</template>

<style scoped>
.mate-popup-trigger {
  display: inline-flex;
  cursor: pointer;
}
.mate-popup-capture {
  position: fixed;
  inset: 0;
  z-index: 1500;
}
.mate-popup-menu {
  position: fixed;
  z-index: 1501;
  min-width: 200px;
  background: rgba(255, 255, 255, 0.96);
  backdrop-filter: blur(16px);
  border-radius: var(--radius-lg);
  box-shadow: var(--sh-pop), 0 0 0 0.5px rgba(0, 0, 0, 0.05);
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.mate-popup-menu__divider {
  height: 1px;
  background: var(--line);
  margin: 5px 10px;
}
.mate-popup-menu__item {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  height: 36px;
  padding: 0 var(--space-md);
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  cursor: pointer;
  font-size: var(--font-body);
  color: var(--ink-900);
  text-align: left;
  white-space: nowrap;
  transition: background-color 0.12s;
}
.mate-popup-menu__item:hover { background-color: var(--bg-fill); }
.mate-popup-menu__item.is-danger { color: var(--err); }
.mate-popup-menu__item.is-danger:hover { background-color: var(--err-bg); }
</style>
