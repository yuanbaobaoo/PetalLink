# PetalLink Sync Resilience and State Consistency Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement one coherent persistent transfer state machine that survives disconnects, preserves the last successful sync baseline, prevents ambiguous duplicate operations, and drives every UI surface from one authoritative state snapshot.

**Architecture:** Extend SQLite to store complete transfer intent and recovery context, then route automatic sync, manual retry, startup recovery, and network recovery through one task executor. Keep `sync_items` as the last confirmed-success baseline; settle successful operations transactionally and derive `SyncGlobalState` from the database with a monotonic revision. Cloud index completeness and dangerous local deletions remain hard safety gates.

**Tech Stack:** Rust 1.77, Tokio, rusqlite, reqwest/rustls, wiremock, Tauri 2, Vue 3, Pinia, TypeScript, Vitest.

## Global Constraints

- macOS 12 or newer; no new remote service or server component.
- Existing databases from schema versions 1 through 4 must migrate in place without deleting sync baselines or transfer history.
- `sync_items.local_path` is always a normalized mount-relative UTF-8 path; `transfer_queue.local_path` remains absolute and `transfer_queue.relative_path` is relative.
- Failed, waiting, ambiguous, canceled, or retried operations never advance `sync_items` mtime, size, editedTime, or fileId.
- Every transfer mutation uses task ID plus state revision; never settle by path or name.
- Existing-file update never falls back to create. Unsupported large-file replacement must fail safely instead of creating a duplicate.
- Incomplete cloud indexes never produce local or cloud deletion actions.
- `sync_state` events are complete authoritative snapshots with monotonic revisions; no default/empty snapshot is used as a notification.
- Network errors wait for recovery; 401 refreshes once; 429/5xx back off; permanent validation/quota/permission errors require user action.
- Use TDD for every behavior change and keep each task independently reviewable.

---

## File and Component Map

- `src/data/migrations.rs`: schema v5 migration and old-row normalization.
- `src/data/repository.rs`: typed task fields, state transitions, task-ID progress/settlement queries, aggregate queries.
- `src/sync/transfer_state.rs` (new): transfer states, operations, error kinds, legal transitions, retry policy.
- `src/sync/status_aggregator.rs` (new): authoritative `SyncGlobalState` computation and revision allocation.
- `src/sync/task_runner.rs` (new): common automatic/manual/startup/network-recovery task execution boundary.
- `src/core/net_guard.rs`: transition notifications and stable online recovery signal.
- `src/drive/client.rs`: structured HTTP error classification and reusable authenticated replay.
- `src/drive/upload_api.rs`: resumable 401 replay, session verification, safe update semantics.
- `src/drive/download_api.rs`: resumable Range downloads and baseline-safe failure behavior.
- `src/drive/changes_api.rs`, `src/drive/files_api.rs`, `src/sync/cloud_tree.rs`: cursor and pagination completeness.
- `src/sync/engine.rs`, `src/sync/executor.rs`, `src/sync/planner.rs`: cycle mutex, task creation, result binding, transactional settlement, trusted-tree deletion gate.
- `src/commands.rs`: safe free-up check, unified retry, queue history semantics, complete state broadcasts.
- `app/api/sync.ts`, `app/api/transfer.ts`, `app/stores/sync.ts`, `app/stores/transfer.ts`: new task/state contracts and revision filtering.
- `app/views/main/SyncStatusBar.vue`, `app/views/main/TransferPopover.vue`: waiting-network and permanent-failure UX.
- `tests/network_resilience_test.rs` (new), existing unit/integration test modules: fault matrix.

---

### Task 1: Schema v5 and Typed Transfer State Model

**Files:**
- Create: `src/sync/transfer_state.rs`
- Modify: `src/sync/mod.rs`
- Modify: `src/data/mod.rs`
- Modify: `src/data/migrations.rs`
- Modify: `src/data/repository.rs`

**Interfaces:**
- Produces: `TransferState`, `TransferOperation`, `TransferErrorKind`, `TransitionError`, `can_transition(from, to)`.
- Produces: expanded `TransferTask` fields and `repository::transition_transfer(conn, task_id, expected_revision, next_state, patch)`.
- Consumes: existing `AppError`, `AppResult`, `rusqlite::Connection`.

- [ ] **Step 1: Write failing transfer-state transition tests**

```rust
#[test]
fn waiting_network_can_resume_but_completed_is_terminal() {
    assert!(can_transition(TransferState::WaitingForNetwork, TransferState::Running));
    assert!(!can_transition(TransferState::Completed, TransferState::Running));
}

#[test]
fn failed_can_be_manually_retried_without_new_task_id() {
    assert!(can_transition(TransferState::Failed, TransferState::Pending));
}
```

- [ ] **Step 2: Run the focused test and verify it fails**

Run: `cargo test --lib sync::transfer_state::tests -- --nocapture`  
Expected: FAIL because `sync::transfer_state` does not exist.

- [ ] **Step 3: Implement the typed state model**

```rust
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferState {
    Pending = 0,
    Running = 1,
    WaitingForNetwork = 2,
    BackingOff = 3,
    VerifyingRemote = 4,
    RestartRequired = 5,
    Completed = 6,
    Failed = 7,
    Canceled = 8,
}

#[repr(i32)]
pub enum TransferOperation {
    Create = 0, Update = 1, Download = 2, DownloadUpdate = 3,
    Delete = 4, Move = 5, Rename = 6, CreateFolder = 7,
}

#[repr(i32)]
pub enum TransferErrorKind {
    Network = 0, Timeout = 1, Auth = 2, RateLimit = 3, Server = 4,
    Quota = 5, Permission = 6, Validation = 7, SessionExpired = 8,
    RemoteAmbiguous = 9, LocalChanged = 10, Unknown = 11,
}
```

- [ ] **Step 4: Write failing v4→v5 and fresh-v5 migration tests**

Assert every new column exists, `PRAGMA user_version == 5`, a legacy RUNNING row becomes Pending, migration is idempotent, and legacy history remains present.

- [ ] **Step 5: Run migration tests and verify failure**

Run: `cargo test --lib data::migrations::tests -- --nocapture`  
Expected: FAIL with schema version 4 or missing `relative_path`.

- [ ] **Step 6: Implement schema v5**

Add columns exactly matching the design, indexes on `(state,next_retry_at)` and `(relative_path,state)`, and migrate old state integers with a single SQL transaction. Set `SCHEMA_VERSION` to 5 in `src/data/mod.rs`.

- [ ] **Step 7: Add repository task-ID transition tests**

```rust
let revision = transition_transfer(&conn, id, 0, TransferState::Running, &TransferPatch::default())?;
assert_eq!(revision, 1);
assert!(transition_transfer(&conn, id, 0, TransferState::Completed, &TransferPatch::default()).is_err());
```

- [ ] **Step 8: Implement expanded row mapping, inserts, and optimistic transitions**

`UPDATE transfer_queue ... WHERE id=? AND state_revision=?` must affect exactly one row; otherwise return a stale-state error.

- [ ] **Step 9: Run focused tests and clippy**

Run: `cargo test --lib data::migrations::tests data::repository::tests sync::transfer_state::tests`  
Expected: PASS.  
Run: `cargo clippy --lib -- -D warnings`  
Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add src/data src/sync/mod.rs src/sync/transfer_state.rs
git commit -m "feat: add persistent transfer state model"
```

---

### Task 2: Authoritative Global State Aggregator

**Files:**
- Create: `src/sync/status_aggregator.rs`
- Modify: `src/sync/mod.rs`
- Modify: `src/sync/state.rs`
- Modify: `src/sync/engine.rs`
- Modify: `src/commands.rs`

**Interfaces:**
- Consumes: v5 task states and `sync_items` successful baselines.
- Produces: `StatusAggregator::snapshot(conn, runtime) -> AppResult<SyncGlobalState>`.
- Produces: `SyncEngine::recompute_and_broadcast_state()`.

- [ ] **Step 1: Write failing aggregation tests**

Create one Completed, one WaitingForNetwork, and one Failed task. Assert `uploading=0`, `waiting_network=1`, `failed=1`, failed details contain only the permanent failure, and successive snapshots have increasing revisions.

- [ ] **Step 2: Run and verify failure**

Run: `cargo test --lib sync::status_aggregator::tests -- --nocapture`  
Expected: FAIL because the aggregator and fields do not exist.

- [ ] **Step 3: Extend the state contract**

```rust
pub struct SyncGlobalState {
    pub revision: u64,
    pub waiting_network: u64,
    pub transfer_failed: u64,
    // retain all existing complete snapshot fields
}
```

Use an `AtomicU64` owned by `StatusAggregator` to allocate revisions.

- [ ] **Step 4: Implement complete DB aggregation**

Count current sync failures separately from historical transfer failures. A transfer in WaitingForNetwork is active/waiting, never permanent failed. Preserve runtime-only fields such as indexing phase by passing a `RuntimeStatus` value.

- [ ] **Step 5: Replace partial/default broadcasts**

Remove every `state_tx.send(SyncGlobalState::default())` and make `push_live_transfer_state` delegate to the authoritative aggregator. Every task transition and history-clear command recomputes a complete snapshot.

- [ ] **Step 6: Add regression test for single retry state consistency**

Start with a relative-path failed sync record and an absolute-path failed task. Transition by task ID and assert the aggregate reports Running then Completed without leaving the failed sync record.

- [ ] **Step 7: Run tests**

Run: `cargo test --lib sync::status_aggregator::tests sync::engine::tests`  
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/sync src/commands.rs
git commit -m "feat: derive authoritative sync status"
```

---

### Task 3: Structured HTTP Error Classification and Recovery Scheduling

**Files:**
- Modify: `src/error.rs`
- Modify: `src/drive/client.rs`
- Modify: `src/core/net_guard.rs`
- Create: `src/sync/retry_policy.rs`
- Modify: `src/sync/mod.rs`

**Interfaces:**
- Produces: `classify_transfer_error(&AppError, operation) -> RecoveryDecision`.
- Produces: `RecoveryDecision::{WaitForNetwork, Backoff{next_retry_at}, RefreshAuth, VerifyRemote, Fail}`.
- Produces: `net_guard::subscribe() -> broadcast::Receiver<NetworkTransition>`.

- [ ] **Step 1: Write table-driven error-policy tests**

Cover connect/timeout, 401, 429 with Retry-After, 503, quota, permission, validation, and ambiguous write timeout.

- [ ] **Step 2: Run and verify failure**

Run: `cargo test --lib sync::retry_policy::tests core::net_guard::tests`  
Expected: FAIL because structured recovery decisions and transitions do not exist.

- [ ] **Step 3: Preserve structured response metadata in AppError**

Ensure status code, Huawei error code, Retry-After, and whether bytes may have been submitted remain available to the policy layer; do not classify by `message.contains("404")`.

- [ ] **Step 4: Implement retry policy with capped exponential backoff**

Use `min(300, 2^attempt)` seconds plus deterministic test-injectable jitter. Network waiting does not increment the permanent retry budget.

- [ ] **Step 5: Emit stable network transitions**

Publish only state changes. Require two consecutive successful probes before Online, and expose a test hook under `cfg(test)` to drive Offline/Online without real sockets.

- [ ] **Step 6: Add flapping-network tests**

Drive Offline→Online→Offline→Online and assert exactly one recovery notification per stable transition.

- [ ] **Step 7: Run focused tests and commit**

Run: `cargo test --lib sync::retry_policy::tests core::net_guard::tests error::tests`  
Expected: PASS.

```bash
git add src/error.rs src/drive/client.rs src/core/net_guard.rs src/sync
git commit -m "feat: classify recoverable transfer failures"
```

---

### Task 4: Unified Task Runner and Retry Path

**Files:**
- Create: `src/sync/task_runner.rs`
- Modify: `src/sync/mod.rs`
- Modify: `src/sync/executor.rs`
- Modify: `src/sync/engine.rs`
- Modify: `src/commands.rs`

**Interfaces:**
- Consumes: `TransferTask`, `RecoveryDecision`, APIs, mount manager.
- Produces: `TaskRunner::run(task_id) -> AppResult<()>`, `TaskRunner::retry(task_id)`, `TaskRunner::resume_waiting()`.
- Produces: one settlement path for automatic execution, manual retry, startup recovery, and online recovery.

- [ ] **Step 1: Write failing precondition tests**

Assert a download retry is validated before changing state, a missing file remains Failed rather than stuck Running, and stale revision attempts cannot mutate the task.

- [ ] **Step 2: Run and verify failure**

Run: `cargo test --lib sync::task_runner::tests -- --nocapture`  
Expected: FAIL because `TaskRunner` does not exist.

- [ ] **Step 3: Implement preflight validation**

Validate operation support, relative/absolute path relation, source metadata, parentId, fileId requirements, and current state before Pending→Running.

- [ ] **Step 4: Route executor enqueue through durable task creation**

Create the task first, retain its ID, and pass that ID to progress, resume, error, and settlement callbacks. Remove all SQL updates keyed by local path or name.

- [ ] **Step 5: Replace the manual retry upload closure**

Delete the separate background implementation in `retry_transfer`. The command calls `TaskRunner::retry(task_id)` and immediately broadcasts the recomputed state.

- [ ] **Step 6: Add upload and download retry tests**

For a subfolder upload assert parentId is preserved. For a download assert the retry executes the download operation rather than returning “only upload supported.”

- [ ] **Step 7: Add startup recovery tests**

Pending/Running tasks become resumable candidates through the same runner; no direct upload occurs before network and source checks.

- [ ] **Step 8: Run tests and commit**

Run: `cargo test --lib sync::task_runner::tests sync::engine::tests sync::executor::tests`  
Expected: PASS.

```bash
git add src/sync src/commands.rs
git commit -m "refactor: unify transfer execution and retry"
```

---

### Task 5: Offline Event Retention, Recovery Wakeup, and Cycle Mutex

**Files:**
- Modify: `src/sync/engine.rs`
- Modify: `src/mount/local_watcher.rs`
- Modify: `src/core/net_guard.rs`

**Interfaces:**
- Consumes: `NetworkTransition::Online`, `TaskRunner::resume_waiting()`.
- Produces: atomic cycle guard and `rescan_required` coalescing.

- [ ] **Step 1: Write paused-time concurrency tests**

Use Tokio barriers to trigger watcher and manual cycles concurrently. Assert one cycle runs and one coalesced follow-up occurs.

- [ ] **Step 2: Write offline-event tests**

Set poll interval to zero, emit a local event while Offline, then Online. Assert a full local rescan and task creation occurs without a second filesystem event.

- [ ] **Step 3: Run and verify failures**

Run: `cargo test --lib sync::engine::tests::network_recovery sync::engine::tests::cycle_mutex`  
Expected: FAIL under the current check-then-set boolean and dropped watcher event behavior.

- [ ] **Step 4: Implement an async cycle mutex and coalesced rescan flag**

Hold one guard across scan→plan→execute→settle. A trigger arriving during the guard sets an atomic flag; the owner runs one follow-up cycle before releasing ownership.

- [ ] **Step 5: Subscribe to stable Online transitions**

Online wakes waiting tasks and sets rescan required immediately. Timer configuration does not affect this path.

- [ ] **Step 6: Run tests and commit**

Run: `cargo test --lib sync::engine::tests mount::local_watcher::tests core::net_guard::tests`  
Expected: PASS.

```bash
git add src/sync/engine.rs src/mount/local_watcher.rs src/core/net_guard.rs
git commit -m "fix: recover coalesced sync work after disconnect"
```

---

### Task 6: Changes Cursor and Cloud Index Completeness

**Files:**
- Modify: `src/drive/changes_api.rs`
- Modify: `src/drive/files_api.rs`
- Modify: `src/sync/cloud_tree.rs`
- Modify: `src/sync/engine.rs`
- Test: `tests/drive_api_test.rs`

**Interfaces:**
- Produces: `list_all_changes(start) -> (Vec<Change>, String)` with final cursor.
- Produces: `list_all` error on cursor loop or configured page safety limit.
- Produces: trusted/incomplete cloud-tree status consumed by planner gates.

- [ ] **Step 1: Add multi-page final-cursor tests**

Mock a nonempty page followed by an empty page and assert the returned cursor is the final `newStartCursor`, not None or the initial cursor.

- [ ] **Step 2: Add cursor-loop and page-limit tests**

Mock a repeated cursor and a final page with nextCursor after the page limit; both must return errors.

- [ ] **Step 3: Run and verify failures**

Run: `cargo test --test drive_api_test changes files_pagination -- --nocapture`  
Expected: FAIL because final cursor is discarded and truncation returns Ok.

- [ ] **Step 4: Implement cursor progress guards and final cursor persistence**

Track seen cursors in a HashSet; reject no-progress pages. Persist the final cursor atomically only after merge succeeds.

- [ ] **Step 5: Propagate incomplete BFS status**

Any child/page failure writes complete=false and keeps the previous trusted in-memory tree. Planner receives `cloud_tree_trusted=false` and filters both deletion directions.

- [ ] **Step 6: Escape search literals**

Add `escape_query_literal` that escapes backslash and quote before URL encoding, with tests for Chinese text, quotes, and backslashes.

- [ ] **Step 7: Run tests and commit**

Run: `cargo test drive::changes_api drive::files_api sync::cloud_tree -- --nocapture`  
Expected: PASS.

```bash
git add src/drive/changes_api.rs src/drive/files_api.rs src/sync/cloud_tree.rs src/sync/engine.rs tests/drive_api_test.rs
git commit -m "fix: enforce complete cloud pagination"
```

---

### Task 7: Safe Upload and Existing-File Update Recovery

**Files:**
- Modify: `src/drive/upload_api.rs`
- Modify: `src/sync/task_runner.rs`
- Modify: `src/sync/executor.rs`
- Modify: `src/drive/files_api.rs`
- Test: `tests/drive_api_test.rs`

**Interfaces:**
- Produces: authenticated chunk replay after 401.
- Produces: `verify_upload_outcome(task) -> RemoteVerification`.
- Produces: safe update failure for unsupported large existing-file replacement.

- [ ] **Step 1: Write failing PATCH timeout regression test**

Simulate a PATCH whose server-side state changes but whose response disconnects. Assert no POST create request is made and remote verification marks the task complete.

- [ ] **Step 2: Write failing large-existing-file test**

Create an Upload action with fileId and size greater than 20 MiB. Assert the executor does not call create/resume POST and returns RestartRequired with an explicit unsupported-update error.

- [ ] **Step 3: Write resumable 401 and lost-308 tests**

Assert one token refresh replays the same session URL and Content-Range; when a chunk response is lost, status query determines the next offset before retransmission.

- [ ] **Step 4: Run and verify failures**

Run: `cargo test --test drive_api_test upload_recovery -- --nocapture`  
Expected: FAIL because PATCH falls back to create and chunks reuse one token without verification.

- [ ] **Step 5: Remove update→create fallback**

Existing fileId always stays Update. Route ambiguous errors to VerifyingRemote and permanent unsupported large updates to RestartRequired.

- [ ] **Step 6: Implement remote create verification**

Search only inside parentId and match name plus size/hash within the task time window. Zero matches permits create retry; one completes; more than one fails RemoteAmbiguous.

- [ ] **Step 7: Implement authenticated chunk replay and session recovery**

Refresh once on 401, query on uncertain network outcomes, and verify target existence before replacing an expired session.

- [ ] **Step 8: Run tests and commit**

Run: `cargo test drive::upload_api sync::task_runner -- --nocapture`  
Expected: PASS.

```bash
git add src/drive/upload_api.rs src/drive/files_api.rs src/sync/task_runner.rs src/sync/executor.rs tests/drive_api_test.rs
git commit -m "fix: make uploads idempotent across disconnects"
```

---

### Task 8: Resumable Download and Baseline-Safe Failure Settlement

**Files:**
- Modify: `src/drive/download_api.rs`
- Modify: `src/sync/task_runner.rs`
- Modify: `src/sync/engine.rs`
- Modify: `src/sync/executor.rs`
- Test: `tests/drive_api_test.rs`

**Interfaces:**
- Produces: download resume metadata stored on task.
- Consumes: expected cloud editedTime and partial-file length.

- [ ] **Step 1: Write failing baseline-preservation test**

Start with a successful baseline, fail a cloud-update download midstream, settle it, and assert the original local mtime and old cloud editedTime remain unchanged.

- [ ] **Step 2: Write Range resume tests**

Mock a partial response failure, retry with `Range: bytes=N-`, and assert append occurs only when the cloud version still matches. A changed version must discard partial data and restart at zero.

- [ ] **Step 3: Run and verify failures**

Run: `cargo test drive::download_api sync::engine::tests::failed_download -- --nocapture`  
Expected: FAIL because current failure settlement advances the cloud baseline and deletes `.tmp`.

- [ ] **Step 4: Implement resumable temp-file metadata and authenticated replay**

Keep the `.tmp` only for recoverable network states; store partial length and expected editedTime. Permanent/canceled tasks remove it.

- [ ] **Step 5: Make failure settlement task-only**

Only successful verified download updates xattrs and `sync_items`. Waiting/Failed changes transfer state and error detail only.

- [ ] **Step 6: Run tests and commit**

Run: `cargo test drive::download_api sync::engine::tests sync::task_runner::tests -- --nocapture`  
Expected: PASS.

```bash
git add src/drive/download_api.rs src/sync/task_runner.rs src/sync/engine.rs src/sync/executor.rs tests/drive_api_test.rs
git commit -m "fix: preserve download baselines across failures"
```

---

### Task 9: Ambiguous Delete, Rename, Move, and Transactional Settlement

**Files:**
- Modify: `src/sync/task_runner.rs`
- Modify: `src/sync/executor.rs`
- Modify: `src/sync/engine.rs`
- Modify: `src/commands.rs`
- Modify: `src/data/repository.rs`

**Interfaces:**
- Produces: operation-specific verification against fileId.
- Produces: `repository::settle_success(transaction, task, verified_file)`.

- [ ] **Step 1: Write ambiguous-write verification tests**

Cover delete committed with lost response, rename committed with lost response, move committed with lost response, and a genuinely uncommitted request.

- [ ] **Step 2: Write transactional rollback test**

Force sync_items settlement to fail and assert transfer_queue does not remain Completed.

- [ ] **Step 3: Run and verify failures**

Run: `cargo test --lib sync::task_runner::tests::ambiguous data::repository::tests::settlement -- --nocapture`  
Expected: FAIL because verification and transactional settlement do not exist.

- [ ] **Step 4: Implement operation-specific verification**

Delete accepts 404/recycled=true; rename compares fileName; move compares first parentFolder. No verification result updates the trusted tree before settlement.

- [ ] **Step 5: Implement one SQLite transaction for task and baseline settlement**

Validate ID/revision/source snapshot, update baseline with real filesystem metadata, mark task Completed, commit, then update in-memory caches and broadcast.

- [ ] **Step 6: Bind action results by stable ID**

Return `(action_id, ActionResult)` from spawned tasks and create a failure result at the same ID for JoinError; never shift result positions.

- [ ] **Step 7: Run tests and commit**

Run: `cargo test --lib sync::task_runner::tests data::repository::tests sync::engine::tests -- --nocapture`  
Expected: PASS.

```bash
git add src/sync src/data/repository.rs src/commands.rs
git commit -m "fix: verify and settle ambiguous remote operations"
```

---

### Task 10: Execution-Time Safety for Free-Up and Deletion

**Files:**
- Modify: `src/commands.rs`
- Modify: `src/sync/engine.rs`
- Modify: `src/sync/planner.rs`
- Modify: `src/core/paths.rs`

**Interfaces:**
- Consumes: authoritative task repository and successful sync baseline.
- Produces: `validate_free_up_now(file_id, relative_path) -> AppResult<FreeUpLease>`.

- [ ] **Step 1: Write TOCTOU free-up test**

Pass the UI precheck, mutate the file mtime/size, call the execution command, and assert the local file remains present with NotSynced.

- [ ] **Step 2: Write active-task and untrusted-tree tests**

An Upload/Update/VerifyingRemote task blocks free-up; an incomplete tree blocks destructive local/cloud delete planning.

- [ ] **Step 3: Run and verify failures**

Run: `cargo test --lib commands::tests::free_up sync::planner::tests::untrusted_tree -- --nocapture`  
Expected: FAIL because execution currently deletes without rechecking.

- [ ] **Step 4: Implement execution-time validation and safe lease**

Re-read DB task state, local metadata, trusted-tree membership, and remote file existence immediately before deletion. Revalidate source metadata after async remote GET before unlinking.

- [ ] **Step 5: Gate planner deletions on trusted tree**

The snapshot carries `cloud_tree_trusted`; false suppresses both DeleteFromLocal and DeleteFromCloud, with diagnostic reasons.

- [ ] **Step 6: Run tests and commit**

Run: `cargo test --lib commands::tests sync::planner::tests sync::engine::tests -- --nocapture`  
Expected: PASS.

```bash
git add src/commands.rs src/sync src/core/paths.rs
git commit -m "fix: revalidate destructive sync operations"
```

---

### Task 11: Frontend Revisioned State and Consistent Retry UX

**Files:**
- Modify: `app/api/sync.ts`
- Modify: `app/api/transfer.ts`
- Modify: `app/stores/sync.ts`
- Modify: `app/stores/transfer.ts`
- Modify: `app/views/main/SyncStatusBar.vue`
- Modify: `app/views/main/TransferPopover.vue`
- Create: `app/stores/sync.test.ts`
- Create: `app/stores/transfer.test.ts`

**Interfaces:**
- Consumes: complete `SyncGlobalState` with revision/waiting_network/transfer_failed.
- Produces: stores that ignore stale snapshots and display retry capability from operation/state.

- [ ] **Step 1: Write stale-revision store test**

```typescript
store.applyState({ ...newer, revision: 8, failed: 0 });
store.applyState({ ...older, revision: 7, failed: 3 });
expect(store.failed).toBe(0);
expect(store.revision).toBe(8);
```

- [ ] **Step 2: Write retry and history semantics tests**

Assert WaitingForNetwork is not permanent failure, transfer history failure is labeled separately, and clearing history does not alter sync failure state.

- [ ] **Step 3: Run and verify failures**

Run: `npm test -- --run app/stores/sync.test.ts app/stores/transfer.test.ts`  
Expected: FAIL because revision and waiting states do not exist.

- [ ] **Step 4: Update TypeScript contracts and stores**

Add exact v5 fields, ignore stale revisions, and keep `transfer_update` limited to queue reload while `sync_state` applies full snapshots.

- [ ] **Step 5: Update UI labels and controls**

Display “等待网络 N” separately, “同步失败 N” from current permanent failures, and “历史失败” inside the queue. Show retry only for Failed/RestartRequired upload or download tasks supported by the runner.

- [ ] **Step 6: Run frontend tests and build**

Run: `npm test`  
Expected: PASS.  
Run: `npm run build`  
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add app
git commit -m "fix: keep transfer and sync UI state consistent"
```

---

### Task 12: End-to-End Fault Matrix and Documentation

**Files:**
- Create: `tests/network_resilience_test.rs`
- Modify: `tests/drive_api_test.rs`
- Modify: `tests/oauth_flow_test.rs`
- Modify: `docs/概要设计文档.md`
- Modify: `docs/api调用整理.md`
- Modify: `README.md`

**Interfaces:**
- Consumes: all previous task interfaces.
- Produces: automated evidence for the approved design completion criteria.

- [ ] **Step 1: Implement deterministic disconnect fixtures**

Create wiremock responders that close before a response, return 401 then success, return 429/Retry-After, return 503 sequences, repeat cursors, and acknowledge a write while disconnecting the client response.

- [ ] **Step 2: Add the complete fault matrix**

For upload, download, update, delete, rename, and move cover pre-request offline, mid-request disconnect, response loss, repeated flapping, token expiry, and process-style recovery from persisted states.

- [ ] **Step 3: Add cross-layer state assertions**

After each scenario assert transfer row, sync baseline, aggregate snapshot, and serialized frontend contract agree. Assert no duplicate create requests and no destructive local deletion from an incomplete tree.

- [ ] **Step 4: Run Rust tests in an environment that permits loopback binding**

Run: `cargo test --lib`  
Expected: all library tests PASS.  
Run: `cargo test --test drive_api_test --test oauth_flow_test --test network_resilience_test`  
Expected: all integration tests PASS.

- [ ] **Step 5: Run static and frontend verification**

Run: `cargo clippy --all-targets -- -D warnings`  
Expected: PASS.  
Run: `npm test` in `app/`  
Expected: PASS.  
Run: `npm run build` in `app/`  
Expected: PASS.  
Run: `git diff --check`  
Expected: no output.

- [ ] **Step 6: Update user and architecture documentation**

Document task states, automatic network recovery, permanent failure semantics, retry behavior, API cursor handling, incomplete-index safety, and the distinction between clearing history and resolving a sync failure.

- [ ] **Step 7: Perform a requirement-by-requirement completion audit**

Map every section of `docs/superpowers/specs/2026-07-12-sync-resilience-and-state-consistency-design.md` to passing tests or inspected implementation evidence. Any missing evidence keeps the work incomplete.

- [ ] **Step 8: Commit**

```bash
git add tests README.md docs
git commit -m "test: cover sync recovery fault matrix"
```

