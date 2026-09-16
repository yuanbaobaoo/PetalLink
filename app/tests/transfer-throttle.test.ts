/**
 * 传输队列节流与渲染上限的单元测试（2026-09-15 传输队列卡死事故的防回归）。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { throttleTrailing } from "@/utils/debounce";
import {
  capTransferHistory,
  TRANSFER_DIR,
  TRANSFER_OPERATION,
  TRANSFER_STATE,
  type TransferState,
  type TransferTask,
} from "@/api/transfer";

/**
 * 构造满足当前测试合同的传输任务。
 */
function task(id: number, state: TransferState): TransferTask {
  return {
    id,
    direction: TRANSFER_DIR.UPLOAD,
    file_id: null,
    local_path: `/mount/task-${id}`,
    name: `task-${id}`,
    total_size: 100,
    transferred: 0,
    state,
    error_message: null,
    created_at: id,
    finished_at: null,
    server_id: null,
    upload_id: null,
    resume_offset: 0,
    session_url: null,
    relative_path: null,
    parent_file_id: null,
    operation: TRANSFER_OPERATION.CREATE,
    source_mtime: null,
    source_size: null,
    expected_cloud_edited_time: null,
    attempt_count: 0,
    next_retry_at: null,
    error_kind: null,
    remote_result_file_id: null,
    state_revision: 0,
    verify_attempt_count: 0,
  };
}

describe("throttleTrailing", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("窗口期内的调用合并为一次拖尾执行", () => {
    const fn = vi.fn();
    const throttled = throttleTrailing(fn, 300);
    throttled();
    throttled();
    throttled();
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(300);
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("持续事件流下不会饿死：每个窗口至多且必然执行一次", () => {
    const fn = vi.fn();
    const throttled = throttleTrailing(fn, 300);
    // 模拟每 100ms 一个事件的洪峰（纯防抖在这种输入下永不执行）。
    for (let i = 0; i < 30; i += 1) {
      vi.advanceTimersByTime(100);
      throttled();
    }
    vi.advanceTimersByTime(300);
    // 3000ms / 300ms ≈ 10 个窗口，允许 ±1 的边界误差。
    const calls = fn.mock.calls.length;
    expect(calls).toBeGreaterThanOrEqual(9);
    expect(calls).toBeLessThanOrEqual(11);
  });
});

describe("capTransferHistory", () => {
  it("非终态任务全量保留，终态历史折叠到上限", () => {
    const tasks: TransferTask[] = [
      task(1, TRANSFER_STATE.RUNNING),
      task(2, TRANSFER_STATE.VERIFYING_REMOTE),
      // 后端按 created_at DESC 返回，靠前者最新；按此约定构造降序 id。
      ...Array.from({ length: 150 }, (_, i) => task(1149 - i, TRANSFER_STATE.COMPLETED)),
    ];
    const { items, hiddenTerminalCount } = capTransferHistory(tasks);

    expect(items.filter((t) => t.state === TRANSFER_STATE.RUNNING)).toHaveLength(1);
    expect(items.filter((t) => t.state === TRANSFER_STATE.VERIFYING_REMOTE)).toHaveLength(1);
    expect(items.filter((t) => t.state === TRANSFER_STATE.COMPLETED)).toHaveLength(100);
    expect(hiddenTerminalCount).toBe(50);
    // 保留的必须是最新的 100 条（id 最大）。
    const keptIds = new Set(items.map((t) => t.id));
    expect(keptIds.has(1149)).toBe(true);
    expect(keptIds.has(1000)).toBe(false);
  });

  it("失败与取消同样计入终态折叠集合", () => {
    const tasks: TransferTask[] = [
      ...Array.from({ length: 60 }, (_, i) => task(2000 + i, TRANSFER_STATE.FAILED)),
      ...Array.from({ length: 60 }, (_, i) => task(3000 + i, TRANSFER_STATE.CANCELED)),
    ];
    const { items, hiddenTerminalCount } = capTransferHistory(tasks);
    expect(items).toHaveLength(100);
    expect(hiddenTerminalCount).toBe(20);
  });

  it("终态不超过上限时不折叠", () => {
    const tasks: TransferTask[] = [
      task(1, TRANSFER_STATE.PENDING),
      task(2, TRANSFER_STATE.COMPLETED),
      task(3, TRANSFER_STATE.FAILED),
    ];
    const { items, hiddenTerminalCount } = capTransferHistory(tasks);
    expect(items).toHaveLength(3);
    expect(hiddenTerminalCount).toBe(0);
  });
});
