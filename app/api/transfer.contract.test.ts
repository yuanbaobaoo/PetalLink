import { describe, expect, it } from "vitest";
import rustTransferStateSource from "../../src/sync/transfer_state.rs?raw";
import { TRANSFER_STATE } from "./transfer";

function rustTransferStateDiscriminants(): Record<string, number> {
  const enumBody = rustTransferStateSource.match(
    /pub enum TransferState\s*\{([\s\S]*?)\n\}/,
  )?.[1];
  if (!enumBody) throw new Error("无法从 Rust 源码读取 TransferState");

  return Object.fromEntries(
    Array.from(enumBody.matchAll(/\b([A-Z][A-Za-z]+)\s*=\s*(\d+)/g), ([, name, value]) => [
      name,
      Number(value),
    ]),
  );
}

describe("TransferState 跨语言合同", () => {
  it("前端 discriminant 与真实 Rust TransferState 逐项一致", () => {
    const rustStates = rustTransferStateDiscriminants();
    const frontendByRustVariant = {
      Pending: TRANSFER_STATE.PENDING,
      Running: TRANSFER_STATE.RUNNING,
      WaitingForNetwork: TRANSFER_STATE.WAITING_FOR_NETWORK,
      BackingOff: TRANSFER_STATE.BACKING_OFF,
      VerifyingRemote: TRANSFER_STATE.VERIFYING_REMOTE,
      RestartRequired: TRANSFER_STATE.RESTART_REQUIRED,
      Completed: TRANSFER_STATE.COMPLETED,
      Failed: TRANSFER_STATE.FAILED,
      Canceled: TRANSFER_STATE.CANCELED,
    };

    expect(frontendByRustVariant).toEqual(rustStates);
  });
});
