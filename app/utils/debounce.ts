/**
 * 通用函数工具。
 */

/**
 * 拖尾节流：窗口期内多次调用合并为一次，窗口结束后执行。
 * 与纯防抖不同，持续事件流下不会饿死——每个窗口至多执行一次且必然执行。
 *
 * @param fn - 要节流的函数
 * @param intervalMs - 最小执行间隔（毫秒）
 * @returns 节流后的调用入口
 */
export function throttleTrailing(fn: () => void, intervalMs: number): () => void {
  // 当前窗口的定时器；非空表示窗口内已排队一次执行。
  let timer: ReturnType<typeof setTimeout> | null = null;
  return () => {
    if (timer !== null) return;
    timer = setTimeout(() => {
      timer = null;
      fn();
    }, intervalMs);
  };
}
