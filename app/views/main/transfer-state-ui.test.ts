// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { flushPromises, shallowMount } from "@vue/test-utils";
import * as transferApi from "@/api/transfer";
import {
  TRANSFER_DIR,
  TRANSFER_STATE,
  type TransferTask,
} from "@/api/transfer";
import { useSyncStore } from "@/stores/sync";
import { useTransferStore } from "@/stores/transfer";
import SyncStatusBar from "./SyncStatusBar.vue";
import TransferPopover from "./TransferPopover.vue";

function task(id: number, state: number, direction: number = TRANSFER_DIR.UPLOAD): TransferTask {
  return {
    id,
    direction,
    name: `task-${id}`,
    total_size: 100,
    transferred: 50,
    state,
    error_message: state === TRANSFER_STATE.FAILED ? "真实失败" : undefined,
    created_at: id,
  };
}

describe("TransferPopover 后端状态呈现", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.restoreAllMocks();
  });

  it("显示 9 状态的准确文案，且只有 Failed 任务出现重试", async () => {
    const tasks = [
      task(1, TRANSFER_STATE.PENDING),
      task(2, TRANSFER_STATE.RUNNING),
      task(3, TRANSFER_STATE.WAITING_FOR_NETWORK),
      task(4, TRANSFER_STATE.BACKING_OFF),
      task(5, TRANSFER_STATE.VERIFYING_REMOTE),
      task(6, TRANSFER_STATE.RESTART_REQUIRED),
      task(7, TRANSFER_STATE.COMPLETED),
      task(8, TRANSFER_STATE.FAILED),
      task(9, TRANSFER_STATE.CANCELED),
      task(10, TRANSFER_STATE.FAILED, TRANSFER_DIR.DELETE),
    ];
    vi.spyOn(transferApi, "listAllTransfers").mockResolvedValue(tasks);

    const wrapper = shallowMount(TransferPopover, {
      global: { plugins: [setActivePinia(createPinia())] },
    });
    await flushPromises();

    for (const label of [
      "等待调度",
      "传输中",
      "等待网络",
      "等待重试",
      "核验远端",
      "等待重新规划",
      "已完成",
      "失败",
      "已取消",
    ]) {
      expect(wrapper.text()).toContain(label);
    }

    const retriableNames = wrapper
      .findAll(".tp-item")
      .filter((item) => item.find(".tp-item__retry").exists())
      .map((item) => item.find(".tp-item__name").text());
    expect(retriableNames).toEqual(expect.arrayContaining([expect.stringContaining("task-8"), expect.stringContaining("task-10")]));
    expect(retriableNames).toHaveLength(2);
  });
});

describe("SyncStatusBar 活动态与失败事实", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.restoreAllMocks();
    const sync = useSyncStore();
    sync.mountConfigured = true;
    vi.spyOn(transferApi, "listAllTransfers").mockImplementation(async () => [
      ...useTransferStore().tasks,
    ]);
  });

  it.each([
    [TRANSFER_STATE.WAITING_FOR_NETWORK, "等待网络恢复…"],
    [TRANSFER_STATE.BACKING_OFF, "等待下次重试…"],
    [TRANSFER_STATE.VERIFYING_REMOTE, "正在核验远端状态…"],
    [TRANSFER_STATE.RESTART_REQUIRED, "等待重新规划…"],
  ])("队列 state=%s 时主页不显示同步完成", (state, expectedText) => {
    const transfer = useTransferStore();
    transfer.tasks = [task(1, state)];

    const wrapper = shallowMount(SyncStatusBar);

    expect(wrapper.text()).toContain(expectedText);
    expect(wrapper.text()).not.toContain("同步完成");
  });

  it("首次渲染主动加载队列后识别 BackingOff", async () => {
    vi.mocked(transferApi.listAllTransfers).mockResolvedValue([
      task(1, TRANSFER_STATE.BACKING_OFF),
    ]);

    const wrapper = shallowMount(SyncStatusBar);
    await flushPromises();

    expect(wrapper.text()).toContain("等待下次重试…");
    expect(wrapper.text()).not.toContain("同步完成");
  });

  it("权威快照 waitingNetwork 使用等待网络文案而非泛化的同步中", () => {
    const sync = useSyncStore();
    sync.applyState({
      total: 1,
      completed: 0,
      uploading: 0,
      downloading: 0,
      waitingNetwork: 1,
      failed: 0,
      transferFailed: 0,
      failed_items: [],
      conflict: 0,
      editing: 0,
      is_running: false,
      last_sync_time: null,
      is_indexing: false,
      indexing_scanned_folders: 0,
      indexing_discovered_items: 0,
      content_changed: false,
    });

    const wrapper = shallowMount(SyncStatusBar);

    expect(wrapper.text()).toContain("等待网络恢复…");
    expect(wrapper.text()).not.toContain("同步完成");
  });

  it("分别显示当前同步失败与历史传输失败", () => {
    const sync = useSyncStore();
    sync.applyState({
      total: 2,
      completed: 1,
      uploading: 0,
      downloading: 0,
      waitingNetwork: 0,
      failed: 1,
      transferFailed: 3,
      failed_items: [{ relative_path: "current.txt" }],
      conflict: 0,
      editing: 0,
      is_running: false,
      last_sync_time: null,
      is_indexing: false,
      indexing_scanned_folders: 0,
      indexing_discovered_items: 0,
      content_changed: false,
    });

    const wrapper = shallowMount(SyncStatusBar);

    expect(wrapper.text()).toContain("同步失败 1");
    expect(wrapper.text()).toContain("历史失败 3");
  });
});
