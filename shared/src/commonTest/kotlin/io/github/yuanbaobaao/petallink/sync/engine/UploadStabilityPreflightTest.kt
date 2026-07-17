package io.github.yuanbaobaao.petallink.sync.engine

import io.github.yuanbaobaao.petallink.auth.TokenPair
import io.github.yuanbaobaao.petallink.data.TransferDirection
import io.github.yuanbaobaao.petallink.drive.DownloadApi
import io.github.yuanbaobaao.petallink.drive.DriveClient
import io.github.yuanbaobaao.petallink.drive.UploadApi
import io.github.yuanbaobaao.petallink.sync.TransferState
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respondError
import io.ktor.http.HttpStatusCode
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs

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

    private fun operations(
        probe: UploadStabilityProbe,
        pause: suspend (Long) -> Unit = {},
    ): TransferOperationsImpl {
        val client = DriveClient(
            HttpClient(MockEngine { respondError(HttpStatusCode.InternalServerError) }),
            { "token" },
            { TokenPair("new", "refresh", Long.MAX_VALUE) },
        )
        return TransferOperationsImpl(
            UploadApi(client), DownloadApi(client),
            readFileBytes = { byteArrayOf() }, writeFileBytes = { _, _ -> },
            fileExists = { true }, fileSize = { 0 },
            uploadStability = probe, stabilityPause = pause,
        )
    }

    private fun task() = TaskContext(
        1, "file", "/tmp/file", TransferDirection.UPLOAD,
        TransferState.Pending, 0, 0, 0, 0,
    )
}
