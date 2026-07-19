package io.github.yuanbaobaoo.petallink.ui.viewmodel

import io.github.yuanbaobaoo.petallink.core.net_guard.NetState
import io.github.yuanbaobaoo.petallink.sync.engine.SyncGlobalStatus
import io.github.yuanbaobaoo.petallink.sync.TransferState
import io.github.yuanbaobaoo.petallink.drive.DriveFile
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * ViewModel 单测（对标 docs/08 §stores）。
 */
class ViewModelsTest {
    private fun task(revision: Long, done: Long) = TransferTaskUi(
        1, "a.txt", TransferState.Running, revision, 100, done, "upload", null,
    )

    @Test
    fun SyncViewModel_同revision重复投递被忽略() {
        val vm = SyncViewModel()
        vm.applyState(SyncGlobalStatus.SYNCING, revision = 1)
        assertEquals(SyncGlobalStatus.SYNCING, vm.state.value)
        // 同 revision 重复投递 → 幂等，不改
        vm.applyState(SyncGlobalStatus.IDLE, revision = 1)
        assertEquals(SyncGlobalStatus.SYNCING, vm.state.value)
        // 新 revision 才生效
        vm.applyState(SyncGlobalStatus.IDLE, revision = 2)
        assertEquals(SyncGlobalStatus.IDLE, vm.state.value)
    }

    @Test
    fun SyncViewModel_旧revision不能回滚状态() {
        val vm = SyncViewModel()
        vm.applyState(SyncGlobalStatus.SYNCING, 9)
        vm.applyState(SyncGlobalStatus.IDLE, 8)
        assertEquals(SyncGlobalStatus.SYNCING, vm.state.value)
    }

    @Test
    fun TransferViewModel_过期请求丢弃() {
        val vm = TransferViewModel()
        val tasks = listOf(TransferTaskUi(1, "a.txt", TransferState.Running, 1, 100, 50, "upload", null))
        vm.loadAll(tasks, requestId = 5)
        assertEquals(1, vm.tasks.value.size)
        // 过期请求（requestId < 5）被丢弃
        vm.loadAll(emptyList(), requestId = 3)
        assertEquals(1, vm.tasks.value.size)
    }

    @Test
    fun TransferViewModel_进度更新乱序保护() {
        val vm = TransferViewModel()
        val task = TransferTaskUi(1, "a.txt", TransferState.Running, stateRevision = 1, 100, 0, "upload", null)
        vm.loadAll(listOf(task), requestId = 1)
        // 正常更新（新 revision）
        vm.updateProgress(1, bytesDone = 80, revision = 2)
        assertEquals(80, vm.tasks.value[0].bytesDone)
        // 旧 revision 被忽略（乱序保护）
        vm.updateProgress(1, bytesDone = 10, revision = 1)
        assertEquals(80, vm.tasks.value[0].bytesDone)
    }

    @Test
    fun TransferViewModel_新列表请求也不能回滚单任务revision() {
        val vm = TransferViewModel()
        vm.loadAll(listOf(task(revision = 8, done = 80)), 1)
        vm.loadAll(listOf(task(revision = 7, done = 70)), 2)
        assertEquals(8, vm.tasks.value.single().stateRevision)
        assertEquals(80, vm.tasks.value.single().bytesDone)
    }

    @Test
    fun FileBrowserViewModel_拒绝旧页且文件夹优先排序() {
        val vm = FileBrowserViewModel()
        val current = vm.beginLoad()
        val later = vm.beginLoad()
        assertTrue(vm.applyPage(later, null, listOf(
            DriveFile(id = "f", name = "z-folder", mimeType = "application/vnd.huawei-apps.folder"),
            DriveFile(id = "b", name = "b.txt", category = "file", size = "2"),
            DriveFile(id = "a", name = "a.txt", category = "file", size = "1"),
        ), null, false))
        assertFalse(vm.applyPage(current, null, emptyList(), null, false))
        assertEquals(listOf("z-folder", "a.txt", "b.txt"), vm.state.value.visibleFiles.map { it.name })
    }

    @Test
    fun FileBrowserViewModel_进入目录保留完整面包屑路径() {
        val vm = FileBrowserViewModel()
        val parent = DriveFile(id = "parent", name = "父目录", mimeType = "application/vnd.huawei-apps.folder")
        val child = DriveFile(id = "child", name = "子目录", mimeType = "application/vnd.huawei-apps.folder")

        vm.enter(parent)
        vm.enter(child)

        assertEquals(listOf(null, "parent", "child"), vm.state.value.breadcrumbs.map { it.id })
        vm.navigateTo(vm.state.value.breadcrumbs.first())
        assertEquals(listOf(null), vm.state.value.breadcrumbs.map { it.id })
    }

    @Test
    fun TransferTaskUi_进度百分比() {
        val task = TransferTaskUi(1, "a", TransferState.Running, 1, 200, 50, "upload", null)
        assertEquals(0.25f, task.progress)
    }

    @Test
    fun TransferTaskUi_零大小进度为零() {
        val task = TransferTaskUi(1, "a", TransferState.Running, 1, 0, 0, "upload", null)
        assertEquals(0f, task.progress)
    }

    @Test
    fun UpdaterViewModel_phase切换() {
        val vm = UpdaterViewModel()
        vm.setPhase(UpdaterPhase.AVAILABLE)
        assertEquals(UpdaterPhase.AVAILABLE, vm.phase.value)
        vm.setNewVersion("1.2.0")
        assertEquals("1.2.0", vm.newVersion.value)
    }
}
