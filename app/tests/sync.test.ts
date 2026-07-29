import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import type { SyncGlobalState } from "@/api/sync";
import * as configApi from "@/api/config";
import { useSyncStore } from "@/stores/sync";

/**
 * 构造可按字段覆盖的同步状态快照。
 */
function snapshot(overrides: Partial<SyncGlobalState> = {}): SyncGlobalState {
  return {
    revision: 1,
    total: 3,
    completed: 2,
    uploading: 0,
    downloading: 0,
    waiting_network: 0,
    failed: 1,
    transfer_failed: 0,
    failed_items: [{ relative_path: "current.txt", error_message: "sync failed" }],
    conflict: 0,
    editing: 0,
    is_running: false,
    last_sync_time: null,
    is_indexing: false,
    indexing_scanned_folders: 0,
    indexing_discovered_items: 0,
    content_changed: false,
    ...overrides,
  };
}

describe("sync store 权威快照字段", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.restoreAllMocks();
  });

  it("接收 waiting_network 并保持等待态为活动中", () => {
    // 当前测试使用的 Store 实例。
    const store = useSyncStore();
    store.applyState(snapshot({ waiting_network: 2 }));

    expect(store.waitingNetwork).toBe(2);
    expect(store.hasActiveTransfer).toBe(true);
  });

  it("同步项 failed 与历史 transferFailed 分开保存", () => {
    // 当前测试使用的 Store 实例。
    const store = useSyncStore();
    store.applyState(snapshot({ failed: 1, transfer_failed: 4 }));

    expect(store.failed).toBe(1);
    expect(store.failedItems).toHaveLength(1);
    expect(store.transferFailed).toBe(4);
  });

  it("拒绝旧 revision 覆盖更新状态", () => {
    // 当前测试使用的 Store 实例。
    const store = useSyncStore();
    store.applyState(snapshot({ revision: 8, failed: 0, failed_items: [] }));
    store.applyState(snapshot({ revision: 7, failed: 3 }));

    expect(store.revision).toBe(8);
    expect(store.failed).toBe(0);
    expect(store.failedItems).toEqual([]);
  });

  it("首次同步运行中且数据库尚为空时使用不确定进度", () => {
    // 当前测试使用的 Store 实例。
    const store = useSyncStore();
    store.applyState(snapshot({
      revision: 2,
      total: 0,
      completed: 0,
      failed: 0,
      failed_items: [],
      is_running: true,
    }));

    expect(store.progress).toBeNull();

    store.applyState(snapshot({
      revision: 3,
      total: 0,
      completed: 0,
      failed: 0,
      failed_items: [],
      is_running: false,
    }));
    expect(store.progress).toBe(1);
  });

  it("配置保存成功后可直接提交权威挂载状态", () => {
    // 当前测试使用的 Store 实例。
    const store = useSyncStore();

    store.applyMountConfiguration("/Users/test/PetalLink");

    expect(store.mountConfigured).toBe(true);
    expect(store.mountDir).toBe("/Users/test/PetalLink");
    expect(store.setupPhase).toBe("active");
  });

  it("已提交挂载事实不因紧随其后的配置读取失败而回退", async () => {
    // 当前测试使用的 Store 实例。
    const store = useSyncStore();
    vi.spyOn(configApi, "loadConfig").mockRejectedValue(new Error("temporary read failure"));
    store.applyMountConfiguration("/Users/test/PetalLink");

    await store.init();

    expect(store.mountConfigured).toBe(true);
    expect(store.mountDir).toBe("/Users/test/PetalLink");
    expect(store.setupPhase).toBe("active");
  });
});
