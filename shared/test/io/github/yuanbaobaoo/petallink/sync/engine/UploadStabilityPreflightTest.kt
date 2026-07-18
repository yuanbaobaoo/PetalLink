package io.github.yuanbaobaoo.petallink.sync.engine

import io.github.yuanbaobaoo.petallink.auth.TokenPair
import io.github.yuanbaobaoo.petallink.data.TransferDirection
import io.github.yuanbaobaoo.petallink.drive.DownloadApi
import io.github.yuanbaobaoo.petallink.drive.DriveClient
import io.github.yuanbaobaoo.petallink.drive.UploadApi
import io.github.yuanbaobaoo.petallink.sync.TransferState
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respondError
import io.ktor.http.HttpStatusCode
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertFailsWith
import io.github.yuanbaobaoo.petallink.AppError

class UploadStabilityPreflightTest {
    @Test
    fun 不稳定上传按_0_2_3_5_秒重试后转RestartRequired() = runBlocking {
        val calls = mutableListOf<Long>()
        var probes = 0
        val operations = operations(
            UploadStabilityProbe { probes++; UploadStability.UNSTABLE },
            pause = { calls += it },
        )
        val result = operations.preflight(task())
        val rejected = assertIs<PreflightResult.Reject>(result)
        assertEquals(TransferState.RestartRequired, rejected.targetState)
        assertEquals(4, probes)
        assertEquals(listOf(2_000L, 3_000L, 5_000L), calls)
    }

    @Test
    fun 第三次稳定立即放行() = runBlocking {
        var probes = 0
        val operations = operations(UploadStabilityProbe {
            probes++
            if (probes == 3) UploadStability.STABLE else UploadStability.UNSTABLE
        })
        assertEquals(PreflightResult.Ok, operations.preflight(task()))
        assertEquals(3, probes)
    }

    @Test
    fun 上传稳定性检查前必须通过云盘配额预检() = runBlocking {
        var required = -1L
        val operations = operations(
            UploadStabilityProbe { UploadStability.STABLE },
            fileSize = 42L,
            ensureCapacity = { required = it },
        )

        assertEquals(PreflightResult.Ok, operations.preflight(task()))
        assertEquals(42L, required)
    }

    @Test
    fun 配额不足必须阻断上传预检() = runBlocking {
        val operations = operations(
            UploadStabilityProbe { UploadStability.STABLE },
            ensureCapacity = { throw AppError.Data("空间不足") },
        )

        assertFailsWith<AppError.Data> { operations.preflight(task()) }
        Unit
    }

    private fun operations(
        probe: UploadStabilityProbe,
        pause: suspend (Long) -> Unit = {},
        fileSize: Long = 0L,
        ensureCapacity: suspend (Long) -> Unit = {},
    ): TransferOperationsImpl {
        val client = DriveClient(
            HttpClient(MockEngine { respondError(HttpStatusCode.InternalServerError) }),
            { "token" },
            { TokenPair("new", "refresh", Long.MAX_VALUE) },
        )
        return TransferOperationsImpl(
            UploadApi(client), DownloadApi(client),
            readFileBytes = { byteArrayOf() }, writeFileBytes = { _, _ -> },
            fileExists = { true }, fileSize = { fileSize },
            uploadStability = probe, stabilityPause = pause,
            ensureUploadCapacity = ensureCapacity,
        )
    }

    private fun task() = TaskContext(
        1, "file", "/tmp/file", TransferDirection.UPLOAD,
        TransferState.Pending, 0, 0, 0, 0,
    )
}
