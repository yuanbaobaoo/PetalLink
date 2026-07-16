package io.github.yuanbaobaao.petallink.drive

import io.github.yuanbaobaao.petallink.AppError
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * UploadProtocol 单测（对标 docs/03 §308 rangeList 连续性校验）。
 */
class UploadProtocolTest {

    @Test
    fun parseConfirmedOffset_空rangeList返回0() {
        assertEquals(0L, UploadProtocol.parseConfirmedOffset(emptyList(), 1000L))
    }

    @Test
    fun parseConfirmedOffset_单个范围从0开始() {
        // "0-99" → 已上传 100 字节，下一偏移 100
        assertEquals(100L, UploadProtocol.parseConfirmedOffset(listOf("0-99"), 1000L))
    }

    @Test
    fun parseConfirmedOffset_连续多范围() {
        // 0-99, 100-199 → 下一偏移 200
        assertEquals(200L, UploadProtocol.parseConfirmedOffset(listOf("0-99", "100-199"), 1000L))
    }

    @Test
    fun parseConfirmedOffset_不连续抛异常() {
        // 0-99, 200-299（中间断档）
        assertFailsWith<AppError.Remote> {
            UploadProtocol.parseConfirmedOffset(listOf("0-99", "200-299"), 1000L)
        }
    }

    @Test
    fun parseConfirmedOffset_不从0开始抛异常() {
        assertFailsWith<AppError.Remote> {
            UploadProtocol.parseConfirmedOffset(listOf("50-99"), 1000L)
        }
    }

    @Test
    fun parseConfirmedOffset_end越界抛异常() {
        // total=100，end=99 是合法的最后一字节；end=100 越界
        assertEquals(100L, UploadProtocol.parseConfirmedOffset(listOf("0-99"), 100L))
        assertFailsWith<AppError.Remote> {
            UploadProtocol.parseConfirmedOffset(listOf("0-100"), 100L)  // end=100 >= total=100
        }
    }

    @Test
    fun parseConfirmedOffset_反转范围抛异常() {
        assertFailsWith<AppError.Remote> {
            UploadProtocol.parseConfirmedOffset(listOf("10-5"), 1000L)  // end < start
        }
    }

    @Test
    fun parseConfirmedOffset_非法格式抛异常() {
        assertFailsWith<AppError.Remote> {
            UploadProtocol.parseConfirmedOffset(listOf("abc"), 1000L)  // 无 '-'
        }
    }

    @Test
    fun parseConfirmedOffset_两个短横线抛异常() {
        assertFailsWith<AppError.Remote> {
            UploadProtocol.parseConfirmedOffset(listOf("0-9-9"), 1000L)
        }
    }

    @Test
    fun validatedChunkSize_零返回默认值() {
        assertEquals(UploadProtocol.DEFAULT_CHUNK_SIZE, UploadProtocol.validatedChunkSize(0L))
    }

    @Test
    fun validatedChunkSize_合法值原样返回() {
        assertEquals(1024L * 1024, UploadProtocol.validatedChunkSize(1024L * 1024))
    }

    @Test
    fun validatedChunkSize_过小抛异常() {
        assertFailsWith<IllegalArgumentException> {
            UploadProtocol.validatedChunkSize(100L)
        }
    }

    @Test
    fun validatedChunkSize_过大抛异常() {
        assertFailsWith<IllegalArgumentException> {
            UploadProtocol.validatedChunkSize(128L * 1024 * 1024)
        }
    }

    @Test
    fun completeUploadFile_匹配返回file() {
        val file = DriveFile(id = "fid", name = "test.txt", size = "100")
        val result = UploadProtocol.completeUploadFile(file, 100L, "test.txt")
        assertEquals(file, result)
    }

    @Test
    fun completeUploadFile_size不匹配返回null() {
        val file = DriveFile(id = "fid", name = "test.txt", size = "200")
        assertNull(UploadProtocol.completeUploadFile(file, 100L, "test.txt"))
    }

    @Test
    fun completeUploadFile_name不匹配返回null() {
        val file = DriveFile(id = "fid", name = "wrong.txt", size = "100")
        assertNull(UploadProtocol.completeUploadFile(file, 100L, "test.txt"))
    }

    @Test
    fun completeUploadFile_id为空返回null() {
        val file = DriveFile(id = "", name = "test.txt", size = "100")
        assertNull(UploadProtocol.completeUploadFile(file, 100L, "test.txt"))
    }
}
