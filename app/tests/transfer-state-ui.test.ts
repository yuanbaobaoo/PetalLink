// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { flushPromises, shallowMount } from "@vue/test-utils";
import * as transferApi from "@/api/transfer";
import {
  TRANSFER_DIR,
  TRANSFER_OPERATION,
  TRANSFER_STATE,
  type TransferDirection,
  type TransferState,
  type TransferTask,
} from "@/api/transfer";
import { useSyncStore } from "@/stores/sync";
import { useTransferStore } from "@/stores/transfer";
import { MateDialog } from "@/components/mate";
import SyncStatusBar from "@/views/main/SyncStatusBar.vue";
import TransferPopover from "@/views/main/TransferPopover.vue";

/**
 * 构造满足当前测试合同的传输任务。
 */
function task(
  id: number,
  state: TransferState,
  direction: TransferDirection = TRANSFER_DIR.UPLOAD,
): TransferTask {
  // 测试任务对应的传输操作。
  const operation = direction === TRANSFER_DIR.DOWNLOAD
    ? TRANSFER_OPERATION.DOWNLOAD
    : direction === TRANSFER_DIR.DOWNLOAD_UPDATE
      ? TRANSFER_OPERATION.DOWNLOAD_UPDATE
      : direction === TRANSFER_DIR.DELETE
        ? TRANSFER_OPERATION.DELETE
        : TRANSFER_OPERATION.CREATE;
  return {
    id,
    direction,
    file_id: direction === TRANSFER_DIR.UPLOAD ? null : `file-${id}`,
    local_path: `/mount/task-${id}`,
    name: `task-${id}`,
    total_size: 100,
    transferred: 50,
    state,
    error_message: state === TRANSFER_STATE.FAILED ? "真实失败" : null,
    created_at: id,
    finished_at: state === TRANSFER_STATE.FAILED ? id + 1 : null,
    server_id: null,
    upload_id: null,
    resume_offset: 0,
    session_url: null,
    relative_path: `task-${id}`,
    parent_file_id: "root",
    operation,
    source_mtime: 1,
    source_size: 100,
    expected_cloud_edited_time: direction === TRANSFER_DIR.UPLOAD ? null : 1,
    attempt_count: 0,
    next_retry_at: null,
    error_kind: null,
    remote_result_file_id: null,
    state_revision: 0,
  };
}

describe("TransferPopover 后端状态呈现", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.restoreAllMocks();
  });

  it("显示 9 状态的准确文案，且只给支持的 Failed/RestartRequired 任务重试", async () => {
    // 当前场景的传输任务集合。
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

    // 当前组件测试包装器。
    const wrapper = shallowMount(TransferPopover, {
      global: { plugins: [setActivePinia(createPinia())] },
    });
    await flushPromises();

    for (const label of [
      "等待开始",
      "传输中",
      "等待网络",
      "等待重试",
      "正在确认结果",
      "需要重新检查",
      "已完成",
      "失败",
      "已取消",
    ]) {
      expect(wrapper.text()).toContain(label);
    }

    // 界面中允许重试的任务名称。
    const retriableNames = wrapper
      .findAll(".tp-item")
      .filter((item) => item.find(".tp-item__retry").exists())
      .map((item) => item.find(".tp-item__name").text());
    expect(retriableNames).toEqual(expect.arrayContaining([
      expect.stringContaining("task-6"),
      expect.stringContaining("task-8"),
    ]));
    expect(retriableNames).toHaveLength(2);
  });
});

describe("SyncStatusBar 活动态与失败事实", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.restoreAllMocks();
    // 当前同步状态。
    const sync = useSyncStore();
    sync.mountConfigured = true;
    vi.spyOn(transferApi, "listAllTransfers").mockImplementation(async () => [
      ...useTransferStore().tasks,
    ]);
  });

  it.each([
    [TRANSFER_STATE.WAITING_FOR_NETWORK, "等待网络恢复…"],
    [TRANSFER_STATE.BACKING_OFF, "等待下次重试…"],
    [TRANSFER_STATE.VERIFYING_REMOTE, "正在确认同步结果…"],
    [TRANSFER_STATE.RESTART_REQUIRED, "有文件需要重新检查…"],
  ] as const)("队列 state=%s 时主页不显示同步完成", (state, expectedText) => {
    // 当前传输队列状态。
    const transfer = useTransferStore();
    transfer.tasks = [task(1, state)];

    // 当前组件测试包装器。
    const wrapper = shallowMount(SyncStatusBar);

    expect(wrapper.text()).toContain(expectedText);
    expect(wrapper.text()).not.toContain("同步完成");
  });

  it("首次渲染主动加载队列后识别 BackingOff", async () => {
    vi.mocked(transferApi.listAllTransfers).mockResolvedValue([
      task(1, TRANSFER_STATE.BACKING_OFF),
    ]);

    // 当前组件测试包装器。
    const wrapper = shallowMount(SyncStatusBar);
    await flushPromises();

    expect(wrapper.text()).toContain("等待下次重试…");
    expect(wrapper.text()).not.toContain("同步完成");
  });

  it.each([
    ["planning-startup", "正在分析首次同步内容…"],
    ["planning-sync", "正在分析同步内容…"],
    ["materializing-local", "正在创建本地目录和占位文件…"],
    ["executing-actions", "正在执行同步操作…"],
  ] as const)("同步阶段 %s 显示精确文案", (phase, expectedText) => {
    // 当前同步状态。
    const sync = useSyncStore();
    sync.syncPhase = phase;
    sync.isRunning = true;

    // 当前组件测试包装器。
    const wrapper = shallowMount(SyncStatusBar);

    expect(wrapper.text()).toContain(expectedText);
    expect(wrapper.text()).not.toContain("同步完成");
  });

  it("运行中缺少精确 phase 时仍不得显示同步完成", () => {
    // 当前同步状态。
    const sync = useSyncStore();
    sync.syncPhase = null;
    sync.isRunning = true;

    // 当前组件测试包装器。
    const wrapper = shallowMount(SyncStatusBar);

    expect(wrapper.text()).toContain("正在同步…");
    expect(wrapper.text()).not.toContain("同步完成");
  });

  it("索引中缺少精确 phase 时仍不得显示同步完成", () => {
    // 当前同步状态。
    const sync = useSyncStore();
    sync.syncPhase = null;
    sync.isIndexing = true;

    // 当前组件测试包装器。
    const wrapper = shallowMount(SyncStatusBar);

    expect(wrapper.text()).toContain("正在同步…");
    expect(wrapper.text()).not.toContain("同步完成");
  });

  it("权威快照 waitingNetwork 使用等待网络文案而非泛化的同步中", () => {
    // 当前同步状态。
    const sync = useSyncStore();
    sync.applyState({
      revision: 1,
      total: 1,
      completed: 0,
      uploading: 0,
      downloading: 0,
      waiting_network: 1,
      failed: 0,
      transfer_failed: 0,
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

    // 当前组件测试包装器。
    const wrapper = shallowMount(SyncStatusBar);

    expect(wrapper.text()).toContain("等待网络恢复…");
    expect(wrapper.text()).not.toContain("同步完成");
  });

  it("主页只显示当前同步失败，历史失败留在传输队列", () => {
    // 当前同步状态。
    const sync = useSyncStore();
    sync.applyState({
      revision: 1,
      total: 2,
      completed: 1,
      uploading: 0,
      downloading: 0,
      waiting_network: 0,
      failed: 1,
      transfer_failed: 3,
      failed_items: [{ relative_path: "current.txt", error_message: null }],
      conflict: 0,
      editing: 0,
      is_running: false,
      last_sync_time: null,
      is_indexing: false,
      indexing_scanned_folders: 0,
      indexing_discovered_items: 0,
      content_changed: false,
    });

    // 当前组件测试包装器。
    const wrapper = shallowMount(SyncStatusBar);

    expect(wrapper.text()).toContain("同步失败 1");
    expect(wrapper.text()).not.toContain("历史失败 3");
  });

  it("权威快照清空失败后自动关闭失败详情并显示同步完成", async () => {
    // 同步状态 store
    const sync = useSyncStore();
    sync.applyState({
      revision: 1,
      total: 1,
      completed: 0,
      uploading: 0,
      downloading: 0,
      waiting_network: 0,
      failed: 1,
      transfer_failed: 0,
      failed_items: [{ relative_path: "failed.txt", error_message: "sync failed" }],
      conflict: 0,
      editing: 0,
      is_running: false,
      last_sync_time: null,
      is_indexing: false,
      indexing_scanned_folders: 0,
      indexing_discovered_items: 0,
      content_changed: false,
    });

    // 同步状态条包装器
    const wrapper = shallowMount(SyncStatusBar);
    await wrapper.find(".chip--err").trigger("click");

    expect(wrapper.findComponent(MateDialog).props("open")).toBe(true);

    sync.applyState({
      revision: 2,
      total: 1,
      completed: 1,
      uploading: 0,
      downloading: 0,
      waiting_network: 0,
      failed: 0,
      transfer_failed: 0,
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
    await wrapper.vm.$nextTick();

    expect(wrapper.findComponent(MateDialog).props("open")).toBe(false);
    expect(wrapper.text()).toContain("同步完成");
  });
});
