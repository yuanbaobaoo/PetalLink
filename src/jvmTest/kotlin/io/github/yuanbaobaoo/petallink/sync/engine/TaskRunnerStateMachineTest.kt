package io.github.yuanbaobaoo.petallink.sync.engine

import io.github.yuanbaobaoo.petallink.data.ColumnPatch
import io.github.yuanbaobaoo.petallink.data.PetalLinkDb
import io.github.yuanbaobaoo.petallink.data.TransferDirection
import io.github.yuanbaobaoo.petallink.data.TransferPatch
import io.github.yuanbaobaoo.petallink.data.TransferTask
import io.github.yuanbaobaoo.petallink.data.repository.IllegalTransferTransitionException
import io.github.yuanbaobaoo.petallink.data.repository.StaleRevisionException
import io.github.yuanbaobaoo.petallink.sync.TransferState
import io.github.yuanbaobaoo.petallink.sync.RetryPolicy
import kotlinx.coroutines.runBlocking
import java.nio.file.Files
import kotlin.io.path.createTempDirectory
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class TaskRunnerStateMachineTest {
    @Test
    fun 自动退避基础窗口固定为1_2_4_8_16秒并只附加有界jitter() {
        assertEquals(listOf(1_000L, 2_000L, 4_000L, 8_000L, 16_000L),
            (0..4).map { RetryPolicy.backoff(it, 0).inWholeMilliseconds })
        assertEquals(16_999L, RetryPolicy.backoff(4, 999).inWholeMilliseconds)
        assertEquals(16_999L, RetryPolicy.backoff(4, 5_000).inWholeMilliseconds)
    }

    @Test
    fun columnPatch与revision在真实SQLite中保持原子语义() = withDb { db ->
        val id = db.transfers.insert(task(errorMessage = "old", nextRetryAt = 88L))
        val running = db.transfers.transition(
            id, 0, TransferState.Running,
            TransferPatch(
                errorMessage = ColumnPatch.Clear,
                nextRetryAt = ColumnPatch.Keep,
                sessionUrl = ColumnPatch.Set("session"),
            ),
        )
        assertNull(running.errorMessage)
        assertEquals(88L, running.nextRetryAt)
        assertEquals("session", running.sessionUrl)

        val backing = db.transfers.transition(
            id, 1, TransferState.BackingOff,
            TransferPatch(
                nextRetryAt = ColumnPatch.Set(2_000L),
                sessionUrl = ColumnPatch.Clear,
                attemptCount = 1,
            ),
        )
        assertEquals(2_000L, backing.nextRetryAt)
        assertNull(backing.sessionUrl)
        assertEquals(1, backing.attempt)
        assertFailsWith<StaleRevisionException> {
            db.transfers.transition(id, 1, TransferState.Running)
        }

        val completed = db.transfers.transition(id, 2, TransferState.Failed)
        assertFailsWith<IllegalTransferTransitionException> {
            db.transfers.transition(id, completed.stateRevision, TransferState.Running)
        }
    }

    @Test
    fun 迟到进度必须同时匹配Running和生命周期revision() = withDb { db ->
        val id = db.transfers.insert(task())
        db.transfers.transition(id, 0, TransferState.Running)
        assertTrue(db.transfers.updateRunningProgress(id, 1, 20L))
        db.transfers.transition(id, 1, TransferState.BackingOff)
        db.transfers.transition(id, 2, TransferState.Running)
        assertFalse(db.transfers.updateRunningProgress(id, 1, 99L))
        assertTrue(db.transfers.updateRunningProgress(id, 3, 30L))
        assertEquals(30L, db.transfers.findById(id)?.transferred)
    }

    @Test
    fun backingOff写入真实截止时间且到期前不执行() = withDb { db ->
        var now = 10_000L
        var executions = 0
        val operations = FakeOperations(execute = {
            executions++
            if (executions == 1) TaskOutput(TaskDisposition.BACKING_OFF, errorMessage = "retry")
            else TaskOutput(TaskDisposition.COMPLETED, bytesTransferred = 10L)
        })
        val runner = TaskRunner(db.transfers, operations, { true }, { now }, { 250L })
        val id = db.transfers.insert(task(totalSize = 10L))

        assertEquals(TaskDisposition.BACKING_OFF, runner.runExpected(context(db, id)))
        val backing = assertNotNull(db.transfers.findById(id))
        assertEquals(1, backing.attempt)
        assertEquals(11_250L, backing.nextRetryAt)
        assertEquals(TaskDisposition.BACKING_OFF, runner.runExpected(context(db, id)))
        assertEquals(1, executions)

        now = 11_250L
        assertEquals(TaskDisposition.COMPLETED, runner.runExpected(context(db, id)))
        assertEquals(2, executions)
    }

    @Test
    fun 启动恢复将Running上传送入核验并保留下载断点() = withDb { db ->
        val uploadId = db.transfers.insert(task(state = TransferState.Running, direction = TransferDirection.UPLOAD))
        val downloadId = db.transfers.insert(
            task(
                state = TransferState.Running,
                direction = TransferDirection.DOWNLOAD,
                totalSize = 100L,
                transferred = 40L,
                resumeOffset = 40L,
            ),
        )
        val runner = TaskRunner(db.transfers, FakeOperations(), { true }, { 5_000L }, { 0L })
        runner.performStartupRecovery {}

        assertEquals(TransferState.VerifyingRemote, db.transfers.findById(uploadId)?.state)
        val download = assertNotNull(db.transfers.findById(downloadId))
        assertEquals(TransferState.Pending, download.state)
        assertEquals(40L, download.resumeOffset)
    }

    @Test
    fun 在线恢复固定先核验再网络等待最后到期退避() = withDb { db ->
        val order = mutableListOf<String>()
        val operations = FakeOperations(
            execute = { task ->
                order += "execute:${task.localPath}"
                TaskOutput(TaskDisposition.COMPLETED)
            },
            verify = {
                order += "verify"
                RemoteVerification.Committed("cloud")
            },
        )
        db.transfers.insert(task(state = TransferState.VerifyingRemote, nextRetryAt = 0L, localPath = "verify"))
        db.transfers.insert(task(state = TransferState.WaitingForNetwork, localPath = "waiting"))
        db.transfers.insert(task(state = TransferState.BackingOff, nextRetryAt = 999L, localPath = "backoff"))
        val runner = TaskRunner(db.transfers, operations, { true }, { 1_000L }, { 0L })

        runner.performOnlineRecovery()

        assertEquals(listOf("verify", "execute:waiting", "execute:backoff"), order)
    }

    @Test
    fun 显式重试只允许四种可安全重放operation() = withDb { db ->
        var executions = 0
        val operations = FakeOperations(execute = { executions++; TaskOutput(TaskDisposition.COMPLETED) })
        val runner = TaskRunner(db.transfers, operations, { true }, { 1_000L }, { 0L })
        val allowed = db.transfers.insert(task(state = TransferState.Failed).copy(operation = 0))
        val rejected = db.transfers.insert(task(state = TransferState.Failed).copy(operation = 9))

        assertEquals(TaskDisposition.COMPLETED, runner.retryExplicit(allowed))
        assertEquals(TaskDisposition.BLOCKED, runner.retryExplicit(rejected))
        assertEquals(1, executions)
        assertEquals(TransferState.Failed, db.transfers.findById(rejected)?.state)
    }

    private fun task(
        state: TransferState = TransferState.Pending,
        direction: TransferDirection = TransferDirection.UPLOAD,
        errorMessage: String? = null,
        nextRetryAt: Long? = null,
        localPath: String = "file.bin",
        totalSize: Long = 0L,
        transferred: Long = 0L,
        resumeOffset: Long = transferred,
    ) = TransferTask(
        id = null,
        direction = direction,
        fileId = "file-id",
        localPath = localPath,
        name = "file.bin",
        totalSize = totalSize,
        transferred = transferred,
        state = state,
        errorMessage = errorMessage,
        createdAt = 1L,
        resumeOffset = resumeOffset,
        nextRetryAt = nextRetryAt,
    )

    private suspend fun context(db: PetalLinkDb, id: Long): TaskContext {
        val task = assertNotNull(db.transfers.findById(id))
        return TaskContext(
            id = id,
            fileId = task.fileId.orEmpty(),
            localPath = task.localPath.orEmpty(),
            direction = task.direction,
            state = task.state,
            stateRevision = task.stateRevision,
            attempt = task.attempt,
            bytesTotal = task.bytesTotal,
            bytesDone = task.bytesDone,
            nextRetryAt = task.nextRetryAt,
            remoteResultFileId = task.remoteResultFileId,
            sessionUrl = task.sessionUrl,
        )
    }

    private fun withDb(block: suspend (PetalLinkDb) -> Unit) = runBlocking {
        val dir = createTempDirectory("petallink-task-runner-")
        val db = PetalLinkDb(dir.resolve("state.db").toString())
        try {
            block(db)
        } finally {
            db.close()
            dir.toFile().deleteRecursively()
        }
    }

    private class FakeOperations(
        private val execute: suspend (TaskContext) -> TaskOutput = {
            TaskOutput(TaskDisposition.COMPLETED)
        },
        private val verify: suspend (TaskContext) -> RemoteVerification = {
            RemoteVerification.Committed(it.fileId)
        },
    ) : TransferOperations {
        override suspend fun preflight(task: TaskContext) = PreflightResult.Ok
        override suspend fun execute(task: TaskContext, progress: TaskProgressReporter) = execute(task)
        override suspend fun verifyRemote(task: TaskContext) = verify(task)
    }
}
