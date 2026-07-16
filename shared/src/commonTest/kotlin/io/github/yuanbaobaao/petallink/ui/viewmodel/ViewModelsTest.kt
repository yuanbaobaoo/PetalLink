package io.github.yuanbaobaao.petallink.ui.viewmodel

import io.github.yuanbaobaao.petallink.core.net_guard.NetState
import io.github.yuanbaobaao.petallink.sync.TransferState
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * ViewModel 单测（对标 docs/08 §stores）。
 */
class ViewModelsTest {

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
