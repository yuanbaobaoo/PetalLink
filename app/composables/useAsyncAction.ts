/**
 * 异步操作封装 —— 统一 loading 状态 + 防重复点击。
 *
 * 需求：所有异步操作按钮（非特殊情况）必须防重复点击 + loading 状态。
 * 用法：
 *   const { loading, run } = useAsyncAction();
 *   async function handleX() { await run(async () => { ... }); }
 *   // 模板：<MateButton :loading="loading" :disabled="loading" @click="handleX" />
 *
 * 行为：loading 期间再次调用直接忽略（防重复点击），finally 复位 loading。
 */
import { ref } from "vue";

export function useAsyncAction<T = void>() {
  const loading = ref(false);

  async function run(fn: () => Promise<T>): Promise<T | undefined> {
    if (loading.value) return undefined; // 防重复点击：并发期间忽略
    loading.value = true;
    try {
      return await fn();
    } finally {
      loading.value = false;
    }
  }

  return { loading, run };
}
