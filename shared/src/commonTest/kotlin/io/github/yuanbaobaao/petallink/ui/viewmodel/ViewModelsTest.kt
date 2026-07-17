package io.github.yuanbaobaao.petallink.ui.viewmodel

import io.github.yuanbaobaao.petallink.core.net_guard.NetState
import io.github.yuanbaobaao.petallink.sync.TransferState
import io.github.yuanbaobaao.petallink.drive.DriveFile
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
        vm.applyState(SyncGlobalState.SYNCING, revision = 1)
        assertEquals(SyncGlobalState.SYNCING, vm.state.value)
        // 同 revision 重复投递 → 幂等，不改
        vm.applyState(SyncGlobalState.IDLE, revision = 1)
        assertEquals(SyncGlobalState.SYNCING, vm.state.value)
        // 新 revision 才生效
        vm.applyState(SyncGlobalState.IDLE, revision = 2)
        assertEquals(SyncGlobalState.IDLE, vm.state.value)
    }

    @Test
    fun SyncViewModel_旧revision不能回滚状态() {
        val vm = SyncViewModel()
        vm.applyState(SyncGlobalState.SYNCING, 9)
        vm.applyState(SyncGlobalState.IDLE, 8)
        assertEquals(SyncGlobalState.SYNCING, vm.state.value)
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

    @Test
    fun AuthViewModel_登录登出() {
        val vm = AuthViewModel()
        assertEquals(false, vm.isLoggedIn.value)
        vm.onLoginSuccess(
            io.github.yuanbaobaao.petallink.auth.TokenPair("at", "rt", 0L),
            UserInfo("张三", "三哥", "138****8888", null),
        )
        assertTrue(vm.isLoggedIn.value)
        assertEquals("张三", vm.userInfo.value?.displayName)
        vm.onLogout()
        assertEquals(false, vm.isLoggedIn.value)
    }
}
