import { beforeEach, describe, expect, it } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import {
  TRANSFER_DIR,
  TRANSFER_OPERATION,
  TRANSFER_STATE,
  type TransferState,
  type TransferTask,
} from "@/api/transfer";
import { useTransferStore } from "@/stores/transfer";

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
    relative_path: `task-${id}`,
    parent_file_id: "root",
    operation: TRANSFER_OPERATION.CREATE,
    source_mtime: 1,
    source_size: 100,
    expected_cloud_edited_time: null,
    attempt_count: 0,
    next_retry_at: null,
    error_kind: null,
    remote_result_file_id: null,
    state_revision: 0,
  };
}

describe("transfer store 状态派生", () => {
  beforeEach(() => setActivePinia(createPinia()));

  it("分别统计 9 个后端状态并保持终态不偏移", () => {
    const store = useTransferStore();
    store.tasks = [
      task(1, TRANSFER_STATE.PENDING),
      task(2, TRANSFER_STATE.RUNNING),
      task(3, TRANSFER_STATE.WAITING_FOR_NETWORK),
      task(4, TRANSFER_STATE.BACKING_OFF),
      task(5, TRANSFER_STATE.VERIFYING_REMOTE),
      task(6, TRANSFER_STATE.RESTART_REQUIRED),
      task(7, TRANSFER_STATE.COMPLETED),
      task(8, TRANSFER_STATE.FAILED),
      task(9, TRANSFER_STATE.CANCELED),
    ];

    expect(store.running).toBe(1);
    expect(store.pending).toBe(1);
    expect(store.waitingNetwork).toBe(1);
    expect(store.backingOff).toBe(1);
    expect(store.verifyingRemote).toBe(1);
    expect(store.restartRequired).toBe(1);
    expect(store.completed).toBe(1);
    expect(store.failed).toBe(1);
    expect(store.canceled).toBe(1);
    expect(store.processing).toBe(2);
    expect(store.waiting).toBe(4);
    expect(store.active).toBe(6);
  });

  it.each([
    ["WaitingForNetwork", TRANSFER_STATE.WAITING_FOR_NETWORK],
    ["BackingOff", TRANSFER_STATE.BACKING_OFF],
    ["VerifyingRemote", TRANSFER_STATE.VERIFYING_REMOTE],
    ["RestartRequired", TRANSFER_STATE.RESTART_REQUIRED],
  ] as const)("%s 仍属于活动任务", (_name, state) => {
    const store = useTransferStore();
    store.tasks = [task(1, state)];

    expect(store.hasActiveTasks).toBe(true);
  });
});
