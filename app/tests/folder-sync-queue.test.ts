// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { flushPromises, mount } from "@vue/test-utils";
import {
  commands,
  FOLDER_SYNC_JOB_STATE,
  TRANSFER_DIR,
  TRANSFER_OPERATION,
  TRANSFER_STATE,
  type FolderSyncJob,
  type TransferTask,
} from "@/api/generated";
import { useFolderSyncStore } from "@/stores/folderSync";
import { toasts } from "@/components/mate/useToast";
import TransferPopover from "@/views/main/TransferPopover.vue";

vi.mock("@/api/generated", async (importOriginal) => {
  // 真实常量与类型保持不变，只替换弹层会调用的命令。
  const actual = await importOriginal<typeof import("@/api/generated")>();
  return {
    ...actual,
    commands: {
      ...actual.commands,
      transferListAll: vi.fn(async () => []),
      transferClearFinished: vi.fn(async () => null),
      syncFolderJobs: vi.fn(async () => []),
      syncFolderClear: vi.fn(async () => 0),
      syncFolderEnqueue: vi.fn(),
      syncFolderCancel: vi.fn(async () => true),
    },
  };
});

/**
 * 构造一条目录同步作业。
 */
function job(overrides: Partial<FolderSyncJob> & { id: number; state: number; rel_path: string }): FolderSyncJob {
  return {
    folder_id: `cloud-${overrides.id}`,
    planned_total: 0,
    enqueued: 0,
    error_message: null,
    created_at: overrides.id,
    started_at: null,
    finished_at: null,
    state_revision: 0,
    ...overrides,
  };
}

/**
 * 构造一条已完成的传输任务。
 */
function completedTask(id: number): TransferTask {
  return {
    id,
    direction: TRANSFER_DIR.UPLOAD,
    file_id: null,
    local_path: `/mount/task-${id}`,
    name: `task-${id}`,
    total_size: 100,
    transferred: 100,
    state: TRANSFER_STATE.COMPLETED,
    error_message: null,
    created_at: id,
    finished_at: id,
    server_id: null,
    upload_id: null,
    resume_offset: 100,
    session_url: null,
    relative_path: `task-${id}`,
    parent_file_id: "root",
    operation: TRANSFER_OPERATION.CREATE,
    source_mtime: 1,
    source_size: 100,
    expected_cloud_edited_time: null,
    attempt_count: 0,
    verify_attempt_count: 0,
    next_retry_at: null,
    error_kind: null,
    remote_result_file_id: null,
    state_revision: 0,
  };
}

/**
 * 挂载传输弹层并等待首次加载完成。
 */
async function mountPopover() {
  // 当前组件测试包装器。
  const wrapper = mount(TransferPopover, {
    global: { plugins: [setActivePinia(createPinia())] },
  });
  await flushPromises();
  return wrapper;
}

describe("目录同步记录并入任务列表", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    toasts.splice(0, toasts.length);
    vi.clearAllMocks();
    vi.mocked(commands.transferListAll).mockResolvedValue([]);
    vi.mocked(commands.transferClearFinished).mockResolvedValue(null);
    vi.mocked(commands.syncFolderJobs).mockResolvedValue([]);
    vi.mocked(commands.syncFolderClear).mockResolvedValue(0);
    vi.mocked(commands.syncFolderCancel).mockResolvedValue(true);
  });

  it("作业与传输任务同列表，并参与四个筛选的计数", async () => {
    vi.mocked(commands.syncFolderJobs).mockResolvedValue([
      job({
        id: 1,
        state: FOLDER_SYNC_JOB_STATE.COMPLETED,
        rel_path: "备份/wedo2.0全套资料",
        planned_total: 2887,
        enqueued: 2887,
      }),
      job({
        id: 2,
        state: FOLDER_SYNC_JOB_STATE.FAILED,
        rel_path: "备份/scratch编程",
        error_message: "读取云端目录失败",
      }),
      job({ id: 3, state: FOLDER_SYNC_JOB_STATE.QUEUED, rel_path: "备份/新目录" }),
    ]);
    vi.mocked(commands.transferListAll).mockResolvedValue([completedTask(4)]);

    // 当前组件测试包装器。
    const wrapper = await mountPopover();

    // 面板叫「任务队列」：目录同步与传输任务都在这份列表里。
    expect(wrapper.find(".tp-header__title").text()).toBe("任务队列");
    // 3 条作业 + 1 条传输任务混排在同一份列表里。
    expect(wrapper.findAll(".tp-item")).toHaveLength(4);
    expect(wrapper.findAll(".tp-item--job")).toHaveLength(3);
    // 全部 / 进行中 / 需要处理 / 已结束 都包含作业。
    expect(wrapper.findAll(".tp-filter__count").map((node) => node.text()))
      .toEqual(["4", "1", "1", "2"]);
    // 失败汇总条与「需要处理」口径一致。
    expect(wrapper.find(".tp-failed-banner").text()).toContain("有 1 项需要处理");
    // 已完成作业的说明仍然可读。
    expect(wrapper.text()).toContain("已投递 2887 项，由传输任务继续执行");
    // 进行中的作业置顶，不会被自己投递出的新任务行挤走。
    expect(wrapper.find(".tp-item__name").text()).toBe("备份/新目录");
    expect(wrapper.find(".tp-item--job .tp-item__tag").text()).toBe("目录同步");

    // 筛选「已结束」：1 条完成作业 + 1 条完成传输。
    await wrapper.findAll(".tp-filter")[3].trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".tp-item")).toHaveLength(2);
    expect(wrapper.findAll(".tp-item--job")).toHaveLength(1);

    // 筛选「进行中」：只剩排队中的作业。
    await wrapper.findAll(".tp-filter")[1].trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".tp-item")).toHaveLength(1);
    expect(wrapper.find(".tp-item__name").text()).toBe("备份/新目录");
  });

  it("失败的作业行可以重新入队", async () => {
    vi.mocked(commands.syncFolderJobs).mockResolvedValue([
      job({
        id: 7,
        state: FOLDER_SYNC_JOB_STATE.FAILED,
        rel_path: "备份/scratch编程",
        error_message: "读取云端目录失败",
      }),
    ]);
    vi.mocked(commands.syncFolderEnqueue).mockResolvedValue(
      job({ id: 8, state: FOLDER_SYNC_JOB_STATE.QUEUED, rel_path: "备份/scratch编程" }),
    );

    // 当前组件测试包装器。
    const wrapper = await mountPopover();
    await wrapper.find(".tp-item--job .tp-item__retry").trigger("click");
    await flushPromises();

    expect(commands.syncFolderEnqueue).toHaveBeenCalledWith("cloud-7", "备份/scratch编程");
    expect(toasts.map((item) => item.message)).toContain("已重新加入目录同步队列：「备份/scratch编程」");
  });

  it("失败的作业行可以取消并保留为已取消历史", async () => {
    vi.mocked(commands.syncFolderJobs).mockResolvedValue([
      job({ id: 9, state: FOLDER_SYNC_JOB_STATE.FAILED, rel_path: "备份/失败目录" }),
    ]);

    // 当前组件测试包装器。
    const wrapper = await mountPopover();
    await wrapper.find(".tp-item--job .tp-item__cancel").trigger("click");
    await flushPromises();

    expect(commands.syncFolderCancel).toHaveBeenCalledWith(9);
    expect(toasts.map((item) => item.message)).toContain("已取消这条失败的目录同步记录");
  });

  it("垃圾桶的「清除全部已结束」同时清理传输任务与目录同步", async () => {
    vi.mocked(commands.syncFolderJobs).mockResolvedValue([
      job({ id: 11, state: FOLDER_SYNC_JOB_STATE.COMPLETED, rel_path: "备份/已完成目录", enqueued: 3 }),
    ]);
    vi.mocked(commands.syncFolderClear).mockResolvedValue(1);

    // 当前组件测试包装器。
    const wrapper = await mountPopover();
    // 打开统计条右侧的清除菜单（菜单被 teleport 到 body）。
    await wrapper.find(".tp-stats__clear").trigger("pointerdown");
    await flushPromises();
    const menuItems = document.body.querySelectorAll(".mate-popup-menu__item");
    expect(menuItems).toHaveLength(3);

    (menuItems[2] as HTMLElement).click();
    await flushPromises();

    // 「清除全部已结束」必须同时覆盖传输历史与目录同步作业。
    expect(commands.transferClearFinished).toHaveBeenCalledTimes(1);
    expect(commands.syncFolderClear).toHaveBeenCalledWith(true, true);
    wrapper.unmount();
  });

  it("清除菜单的两个开关原样传给目录同步清理", async () => {
    // 当前测试使用的 Store 实例。
    const store = useFolderSyncStore();
    vi.mocked(commands.syncFolderClear).mockResolvedValue(2);

    expect(await store.clear(true, false)).toBe(2);
    expect(commands.syncFolderClear).toHaveBeenCalledWith(true, false);

    await store.clear(false, true);
    expect(commands.syncFolderClear).toHaveBeenLastCalledWith(false, true);
  });

  it("零投递的完成回执不再指向传输任务", async () => {
    vi.mocked(commands.syncFolderJobs).mockResolvedValue([
      job({ id: 5, state: FOLDER_SYNC_JOB_STATE.COMPLETED, rel_path: "空目录" }),
    ]);

    // 当前组件测试包装器。
    const wrapper = await mountPopover();

    expect(wrapper.find(".tp-item__detail").text()).toBe("两端已一致，无需传输");
  });
});
