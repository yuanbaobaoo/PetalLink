// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { flushPromises, mount } from "@vue/test-utils";
import * as transferApi from "@/api/transfer";
import {
  TRANSFER_DIR,
  TRANSFER_OPERATION,
  TRANSFER_STATE,
  type RetryAllSummary,
  type RetryEnqueued,
  type TransferTask,
} from "@/api/transfer";
import { commands } from "@/api/generated";
import { showToast, toasts } from "@/components/mate/useToast";
import TransferPopover from "@/views/main/TransferPopover.vue";

vi.mock("@/api/generated", async (importOriginal) => {
  // 真实常量与类型保持不变，只替换两个重试命令的实现。
  const actual = await importOriginal<typeof import("@/api/generated")>();
  return {
    ...actual,
    commands: {
      ...actual.commands,
      transferRetry: vi.fn(),
      syncRetryFailed: vi.fn(),
    },
  };
});

/**
 * 构造一个可手动结算的 Promise，用于观察请求提交期间的 UI 状态。
 */
function createDeferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  // 外部可调用的结算函数。
  let resolve: (value: T) => void = () => {};
  // 手动结算的 Promise。
  const promise = new Promise<T>((inner) => {
    resolve = inner;
  });
  return { promise, resolve };
}

/**
 * 构造一个可重试的失败上传任务。
 */
function failedTask(id: number): TransferTask {
  return {
    id,
    direction: TRANSFER_DIR.UPLOAD,
    file_id: null,
    local_path: `/mount/task-${id}`,
    name: `task-${id}`,
    total_size: 100,
    transferred: 0,
    state: TRANSFER_STATE.FAILED,
    error_message: "真实失败",
    created_at: id,
    finished_at: id,
    server_id: null,
    upload_id: null,
    resume_offset: 0,
    session_url: null,
    relative_path: `task-${id}`,
    parent_file_id: "root",
    operation: TRANSFER_OPERATION.CREATE,
    source_mtime: 1,
    source_size: 100,
    expected_cloud_edited_time: null,
    attempt_count: 1,
    verify_attempt_count: 0,
    next_retry_at: null,
    error_kind: null,
    remote_result_file_id: null,
    state_revision: 0,
  };
}

describe("TransferPopover 重试交互", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.restoreAllMocks();
  });

  it("单条重试只 loading 本行，不阻塞其它失败项", async () => {
    vi.spyOn(transferApi, "listAllTransfers").mockResolvedValue([
      failedTask(1),
      failedTask(2),
    ]);
    // 第一条重试请求保持挂起，用于观察提交期间的按钮状态。
    const pendingRetry = createDeferred<RetryEnqueued>();
    vi.mocked(commands.transferRetry).mockReturnValueOnce(pendingRetry.promise);

    // 当前组件测试包装器。
    const wrapper = mount(TransferPopover, {
      global: { plugins: [setActivePinia(createPinia())] },
    });
    await flushPromises();

    // 两条失败项各自渲染重试按钮。
    const retryButtons = wrapper.findAll(".tp-item__retry");
    expect(retryButtons).toHaveLength(2);

    await retryButtons[0].trigger("click");
    await flushPromises();

    expect(retryButtons[0].attributes("disabled")).toBeDefined();
    expect(retryButtons[1].attributes("disabled")).toBeUndefined();

    pendingRetry.resolve({ Queued: { task_id: 1 } });
    await flushPromises();
  });

  it("一键重试只 loading 自身，行内按钮保持可用", async () => {
    vi.spyOn(transferApi, "listAllTransfers").mockResolvedValue([
      failedTask(1),
      failedTask(2),
    ]);
    // 批量重试请求保持挂起，用于观察提交期间的按钮状态。
    const pendingRetryAll = createDeferred<RetryAllSummary>();
    vi.mocked(commands.syncRetryFailed).mockReturnValueOnce(pendingRetryAll.promise);

    // 当前组件测试包装器。
    const wrapper = mount(TransferPopover, {
      global: { plugins: [setActivePinia(createPinia())] },
    });
    await flushPromises();

    // 失败汇总条必须提供一键入口。
    const retryAllButton = wrapper.find(".tp-failed-banner button");
    expect(retryAllButton.exists()).toBe(true);

    await retryAllButton.trigger("click");
    await flushPromises();

    expect(retryAllButton.attributes("disabled")).toBeDefined();
    for (const button of wrapper.findAll(".tp-item__retry")) {
      expect(button.attributes("disabled")).toBeUndefined();
    }

    pendingRetryAll.resolve({ queued: 2, already_queued: 0, skipped: 0, rejected: 0 });
    await flushPromises();
  });
});

describe("Toast 队列语义", () => {
  beforeEach(() => {
    toasts.splice(0, toasts.length);
  });

  it("连续提示互不覆盖，只挤掉最早的溢出条目", () => {
    showToast("第一条", { variant: "error" });
    showToast("第二条", { variant: "error" });
    showToast("第三条", { variant: "error" });

    expect(toasts.map((item) => item.message)).toEqual(["第一条", "第二条", "第三条"]);

    showToast("第四条", { variant: "error" });

    // 同屏最多 3 条：最早的一条被挤掉，其余保留。
    expect(toasts.map((item) => item.message)).toEqual(["第二条", "第三条", "第四条"]);
  });
});
