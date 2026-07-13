import { beforeEach, describe, expect, it } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import type { SyncGlobalState } from "@/api/sync";
import { useSyncStore } from "./sync";

function snapshot(overrides: Partial<SyncGlobalState> = {}): SyncGlobalState {
  return {
    total: 3,
    completed: 2,
    uploading: 0,
    downloading: 0,
    waitingNetwork: 0,
    failed: 1,
    transferFailed: 0,
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
  beforeEach(() => setActivePinia(createPinia()));

  it("接收 camelCase waitingNetwork 并保持等待态为活动中", () => {
    const store = useSyncStore();
    store.applyState(snapshot({ waitingNetwork: 2 }));

    expect(store.waitingNetwork).toBe(2);
    expect(store.hasActiveTransfer).toBe(true);
  });

  it("同步项 failed 与历史 transferFailed 分开保存", () => {
    const store = useSyncStore();
    store.applyState(snapshot({ failed: 1, transferFailed: 4 }));

    expect(store.failed).toBe(1);
    expect(store.failedItems).toHaveLength(1);
    expect(store.transferFailed).toBe(4);
  });

  it("兼容固定 HEAD 尚未 camelCase 化的新增字段", () => {
    const store = useSyncStore();
    const legacyWire = snapshot() as SyncGlobalState & {
      waiting_network?: number;
      transfer_failed?: number;
    };
    delete (legacyWire as Partial<SyncGlobalState>).waitingNetwork;
    delete (legacyWire as Partial<SyncGlobalState>).transferFailed;
    legacyWire.waiting_network = 2;
    legacyWire.transfer_failed = 5;

    store.applyState(legacyWire);

    expect(store.waitingNetwork).toBe(2);
    expect(store.transferFailed).toBe(5);
  });
});
