package io.github.yuanbaobaao.petallink.sync

import io.github.yuanbaobaao.petallink.config.AppConfig
import io.github.yuanbaobaao.petallink.drive.DriveFile

/**
 * 同步规划器（对标 src/sync/planner.rs）。
 *
 * 详见 docs/06 §planner。
 * decide() 对每个 path 做三方 diff（local/cloud/base）产出 [SyncAction]。
 * plan() 遍历全部 path，应用不可信删除抑制 + Skip 过滤。
 *
 * 关键不变量：
 * - pending: 前缀的 DB 行表示上传待确认（上次可能实际成功）
 * - is_local_changed：mtime 或 size 变化（文件夹 mtime=null → 视为已变）
 * - is_cloud_changed：editedTime 变化（null → 视为未变，防误下载）
 */
object Planner {

    /** pending fileId 前缀（对标 PENDING_FILE_ID_PREFIX） */
    private const val PENDING_PREFIX = "pending:"

    /**
     * 判定本地是否已修改（对标 is_local_changed）。
     * - db.localMtime == null → true（首次记录/文件夹）
     * - mtime 变化 → true
     * - size 变化 → true
     */
    fun isLocalChanged(local: LocalEntry, db: DbBaselineEntry): Boolean {
        if (db.localMtime == null) return true
        if (local.mtime != db.localMtime) return true
        if (db.localSize != null && local.size != db.localSize) return true
        return false
    }

    /**
     * 判定云端是否已修改（对标 is_cloud_changed）。
     * - cloud editedTime 缺失 → false（无信息，视为未变）
     * - db.cloudEditedTimeMs == null → true（首次记录）
     * - editedTime 变化 → true
     */
    fun isCloudChanged(cloudEditedTimeMs: Long?, db: DbBaselineEntry): Boolean {
        if (cloudEditedTimeMs == null) return false
        if (db.cloudEditedTimeMs == null) return true
        return cloudEditedTimeMs != db.cloudEditedTimeMs
    }

    /**
     * 单个 path 的决策（对标 decide()，23 个分支）。
     *
     * @return SyncAction? （null = 无动作）
     */
    fun decide(
        relativePath: String,
        local: LocalEntry?,
        cloud: DriveFile?,
        db: DbBaselineEntry?,
        cloudTreeTrusted: Boolean,
        isStartupResume: Boolean,
    ): SyncAction? {
        val localExists = local != null
        val localHasContent = local != null && local.hasContent
        val cloudExists = cloud != null
        val dbExists = db != null

        // --- A. 文件夹分支 ---
        if (cloud?.isFolder() == true) {
            return decideFolder(relativePath, local, cloud, db, localExists, dbExists, isStartupResume)
        }

        // --- B. 全缺席 ---
        if (!localExists && !cloudExists && !dbExists) return null

        // --- C. 三方都存在（文件） ---
        if (localHasContent && cloudExists && dbExists && db != null) {
            return decideThreeWayPresent(relativePath, local!!, cloud!!, db)
        }

        // --- D. 双方都有但无 DB ---
        if (localExists && cloudExists && !dbExists) {
            // 由 reconcile 补 DB，planner 跳过
            return null
        }

        // --- E. 本地有、云端无 ---
        if (localExists && !cloudExists) {
            return decideLocalOnly(relativePath, local!!, db, dbExists, isStartupResume)
        }

        // --- F. 本地无、云端有 ---
        if (!localExists && cloudExists) {
            return decideCloudOnly(relativePath, cloud!!, db, dbExists, isStartupResume)
        }

        // --- G. 双方都无、DB 残留 ---
        if (!localExists && !cloudExists && dbExists) {
            // engine 周期末尾清 DB 残余
            return null
        }

        return null
    }

    // --- A. 文件夹分支 ---
    private fun decideFolder(
        path: String, local: LocalEntry?, cloud: DriveFile, db: DbBaselineEntry?,
        localExists: Boolean, dbExists: Boolean, isStartupResume: Boolean,
    ): SyncAction? {
        if (!localExists) {
            // 本地无文件夹
            if (dbExists && !isStartupResume) {
                // 会话内删除 → 同步删云端
                return SyncAction(SyncActionType.DELETE_FROM_CLOUD, path, cloud.id, cloud, "会话内本地删除目录 → 同步删除云端")
            }
            if (dbExists && isStartupResume && db?.status == SyncStatus.DELETED) {
                return null  // 启动恢复 + DELETED 墓碑 → 跳过
            }
            // 云端文件夹 → 本地创建
            return SyncAction(SyncActionType.CREATE_FOLDER, path, cloud.id, cloud, "云端文件夹 → 本地创建")
        }
        // 双方都有文件夹 → skip
        return null
    }

    // --- C. 三方都存在 ---
    private fun decideThreeWayPresent(
        path: String, local: LocalEntry, cloud: DriveFile, db: DbBaselineEntry,
    ): SyncAction? {
        // pending 占位项发现云端已有 → 收敛
        if (db.fileId.startsWith(PENDING_PREFIX)) {
            return SyncAction(SyncActionType.SKIP, path, cloud.id, cloud, "pending 占位项发现云端已有 → 收敛为已同步")
        }
        val localChanged = isLocalChanged(local, db)
        val cloudChanged = isCloudChanged(cloudEditedTimeMs(cloud), db)
        return when {
            localChanged && cloudChanged ->
                SyncAction(SyncActionType.CREATE_CONFLICT_COPY, path, cloud.id, cloud, "三方都存在，本地/云端均已修改 → 冲突")
            localChanged && !cloudChanged ->
                SyncAction(SyncActionType.UPLOAD, path, db.fileId, cloud, "本地已修改 → 上传")
            !localChanged && cloudChanged ->
                SyncAction(SyncActionType.DOWNLOAD, path, cloud.id, cloud, "云端已修改 → 下载")
            else -> null  // 未变化
        }
    }

    // --- E. 本地有、云端无 ---
    private fun decideLocalOnly(
        path: String, local: LocalEntry, db: DbBaselineEntry?, dbExists: Boolean, isStartupResume: Boolean,
    ): SyncAction? {
        if (dbExists && db != null) {
            // pending 检查
            if (db.fileId.startsWith(PENDING_PREFIX)) {
                return if (db.status == SyncStatus.FAILED) {
                    null  // FAILED → 不自动重试
                } else {
                    SyncAction(SyncActionType.UPLOAD, path, null, null, "pending 占位项（上传待重试）→ 重新上传")
                }
            }
            // 启动恢复期删除守卫
            if (isStartupResume && !isLocalChanged(local, db)) {
                return SyncAction(SyncActionType.SKIP, path, db.fileId, null, "启动恢复期 cloud_tree 不可信，跳过删除待复核")
            }
            // 类型分发
            return when {
                local.isFolder ->
                    SyncAction(SyncActionType.DELETE_FROM_LOCAL, path, db.fileId, null, "云端已删除文件夹 → 同步删除本地")
                local.hasContent && isLocalChanged(local, db) ->
                    SyncAction(SyncActionType.BACKUP_BEFORE_CLOUD_DELETE, path, db.fileId, null, "云端已删除但本地有未上传修改 → 备份副本")
                else ->
                    SyncAction(SyncActionType.DELETE_FROM_LOCAL, path, db.fileId, null, "云端已删除 → 删除本地")
            }
        }
        // 无 DB
        return when {
            !local.hasContent ->  // 孤儿占位符
                SyncAction(SyncActionType.DELETE_FROM_LOCAL, path, null, null, "孤儿占位符 → 清理")
            local.isFolder ->
                SyncAction(SyncActionType.CREATE_FOLDER, path, null, null, "本地新增文件夹 → 创建云端文件夹")
            else ->
                SyncAction(SyncActionType.UPLOAD, path, null, null, "本地新文件 → 上传")
        }
    }

    // --- F. 本地无、云端有 ---
    private fun decideCloudOnly(
        path: String, cloud: DriveFile, db: DbBaselineEntry?, dbExists: Boolean, isStartupResume: Boolean,
    ): SyncAction? {
        if (dbExists) {
            if (!isStartupResume) {
                return SyncAction(SyncActionType.DELETE_FROM_CLOUD, path, cloud.id, cloud, "会话内删除 → 双向删除云端")
            }
            if (db?.status == SyncStatus.DELETED) {
                return SyncAction(SyncActionType.SKIP, path, null, null, "用户已删除（tombstone）→ 跳过")
            }
            // 启动恢复 + 非 DELETED → 重建占位
        }
        return SyncAction(SyncActionType.CREATE_PLACEHOLDER, path, cloud.id, cloud, "云端新文件 → 创建占位")
    }

    /**
     * 从 DriveFile 提取 editedTime 毫秒。
     * 华为 Drive API 返回 RFC3339 格式的 editedTime 字符串。
     */
    private fun cloudEditedTimeMs(cloud: DriveFile): Long? {
        val raw = cloud.modifiedTime ?: return null
        return try {
            java.time.Instant.parse(raw).toEpochMilli()
        } catch (e: Throwable) {
            null
        }
    }

    /**
     * 生成完整计划（对标 plan()）。
     * 应用不可信删除抑制 + Skip 过滤。
     */
    fun plan(snapshot: SyncSnapshot): List<SyncAction> {
        val allPaths = (snapshot.local.keys + snapshot.cloud.keys + snapshot.db.keys).toSortedSet()
        val actions = mutableListOf<SyncAction>()
        for (path in allPaths) {
            val action = decide(
                path,
                snapshot.local[path],
                snapshot.cloud[path],
                snapshot.db[path],
                snapshot.cloudTreeTrusted,
                snapshot.isStartupResume,
            ) ?: continue

            // 不可信删除抑制
            if (!snapshot.cloudTreeTrusted &&
                (action.type == SyncActionType.DELETE_FROM_LOCAL || action.type == SyncActionType.DELETE_FROM_CLOUD)
            ) {
                continue  // 云端 checkpoint 不可信，抑制删除
            }

            // Skip 过滤：无 cloud_file 的 Skip 丢弃（pending 收敛例外）
            if (action.type == SyncActionType.SKIP && action.cloudFile == null) {
                continue
            }

            actions += action
        }
        return actions
    }
}

/** DriveFile 是否为文件夹（4 种 mimeType 变体，踩坑 1） */
fun DriveFile.isFolder(): Boolean {
    return io.github.yuanbaobaao.petallink.drive.DriveParsers.isFolderMime(mimeType)
}
