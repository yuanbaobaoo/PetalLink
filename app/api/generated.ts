// 此文件由 Tauri Specta 自动生成，禁止手动修改。

import { invoke as __TAURI_INVOKE } from "./tauri";
import * as __TAURI_EVENT from "@tauri-apps/api/event";

// 命令
export const commands = {
	// 检查 OAuth 客户端标识与密钥是否同时完成配置。
	authCheckSecret: () => __TAURI_INVOKE<boolean>("auth_check_secret"),
	// 从 token store 恢复登录状态，并返回当前认证配置快照。
	authRestore: () => __TAURI_INVOKE<AuthState>("auth_restore"),
	// 完成 OAuth 登录，并在切换账号后停止旧运行时、清理同步数据和重置目录配置。
	authLogin: (port: number, sheetBounds: {
	// 左边距。
	x: number,
	// 上边距。
	y: number,
	// 宽度。
	width: number,
	// 高度。
	height: number,
	// 整卡高度（含标题区），用于换算滑入行程。
	// 前端整卡从「卡片高度」的下方滑入，原生视图必须走同样长的距离，
	// 否则动画中登录页会短暂压住标题区。
	sheetHeight: number,
	// 窗口 CSS 视口高度（前端 `window.innerHeight`）。
	// 原生子视图以外框顶边为原点定位，而 CSS 视口从内容区开始，两者相差一个标题栏。
	// 这个偏移量只能由「平台报告的内容区高度 − 真实视口高度」求得：macOS 上平台把
	// 标题栏算进了内容区（`inner_size` == 外框尺寸），单看平台数值无法区分。
	viewportHeight: number,
} | null) => __TAURI_INVOKE<TokenPair>("auth_login", { port, sheetBounds }),
	// 取消正在等待本地回调的 OAuth 授权流程。
	authCancelLogin: () => __TAURI_INVOKE<null>("auth_cancel_login"),
	// 用系统浏览器继续当前授权流程（内嵌登录被拦截时的回退入口）。
	// 只在有进行中的授权流程时有效；返回是否真的打开了浏览器。
	authLoginInBrowser: () => __TAURI_INVOKE<boolean>("auth_login_in_browser"),
	// 窗口尺寸变化时同步底部抽屉的位置与尺寸。
	authLoginSheetBounds: (bounds: SheetBounds) => __TAURI_INVOKE<null>("auth_login_sheet_bounds", { bounds }),
	// 停止同步运行时，清理当前账号的同步数据与目录配置，然后删除登录 token。
	authLogout: () => __TAURI_INVOKE<null>("auth_logout"),
	// 使用当前认证信息读取账号资料。
	authGetUserInfo: () => __TAURI_INVOKE<UserInfo>("auth_get_user_info"),
	// 以本地 token store 是否存在有效记录判断登录状态。
	authIsLoggedIn: () => __TAURI_INVOKE<boolean>("auth_is_logged_in"),
	// 分页列出云盘目录内容。
	driveList: (parentId: string | null, cursor: string | null, pageSize: number | null) => __TAURI_INVOKE<FileListResult>("drive_list", { parentId, cursor, pageSize }),
	// 列出云盘目录的全部内容。
	driveListAll: (parentId: string | null) => __TAURI_INVOKE<DriveFile[]>("drive_list_all", { parentId }),
	// 获取云盘文件信息。
	driveGetFile: (id: string) => __TAURI_INVOKE<DriveFile>("drive_get_file", { id }),
	// 创建云盘目录。
	driveCreateFolder: (name: string, parentId: string | null) => __TAURI_INVOKE<DriveFile>("drive_create_folder", { name, parentId }),
	// 删除云盘文件并结算本地同步状态。
	driveDeleteFile: (id: string, name: string | null) => __TAURI_INVOKE<null>("drive_delete_file", { id, name }),
	// 重命名云盘文件并结算本地路径。
	driveRenameFile: (id: string, newName: string) => __TAURI_INVOKE<DriveFile>("drive_rename_file", { id, newName }),
	// 移动云盘文件并结算本地路径。
	driveMoveFile: (id: string, newParentFolder: string) => __TAURI_INVOKE<DriveFile>("drive_move_file", { id, newParentFolder }),
	// 搜索云盘文件。
	driveSearch: (keyword: string, parentId: string | null, pageSize: number | null) => __TAURI_INVOKE<FileListResult>("drive_search", { keyword, parentId, pageSize }),
	// 搜索云盘文件并自动翻页，返回全部匹配项。
	// 华为服务端的 `fileName contains` 只做**前缀**匹配（官方文档定义），且实测对
	// 中文名可能直接返回空数组；因此这里以「本地/云树子串匹配」为主：
	// 1. 目录内搜索：列出该目录全部子项后本地做子串匹配（精确、只发一次分页）；
	// 2. 整盘搜索：优先在同步引擎的云树快照里匹配（整盘权威路径快照）；
	// 3. 引擎未就绪或快照为空：退回服务端 DSL（前缀语义，结果可能少于预期）。
	driveSearchAll: (keyword: string, parentId: string | null) => __TAURI_INVOKE<DriveFile[]>("drive_search_all", { keyword, parentId }),
	// 获取云盘文件缩略图。
	driveGetThumbnail: (fileId: string) => __TAURI_INVOKE<string>("drive_get_thumbnail", { fileId }),
	// 获取云盘容量信息。
	driveGetAbout: () => __TAURI_INVOKE<DriveAbout>("drive_get_about"),
	// 下载云盘文件到挂载目录。
	driveDownloadFile: (fileId: string, destPath: string) => __TAURI_INVOKE<null>("drive_download_file", { fileId, destPath }),
	// 上传挂载目录中的本地文件。
	driveUploadFile: (localPath: string, parentId: string | null) => __TAURI_INVOKE<DriveFile>("drive_upload_file", { localPath, parentId }),
	// 拖拽导入外部文件/目录到指定同步文件夹（相对挂载根路径，根目录为空串）。
	// 逐源独立复制，任一源失败不阻塞其余源；全部复制完后后台触发一次全量同步周期，
	// command 不等待上传完成即返回，传输进度经传输队列实时可见。
	driveImportFiles: (sourcePaths: string[], targetRelPath: string) => __TAURI_INVOKE<ImportFilesResult>("drive_import_files", { sourcePaths, targetRelPath }),
	// 触发云端树全量刷新与同步周期。
	syncManualRefresh: () => __TAURI_INVOKE<null>("sync_manual_refresh"),
	// 检查文件是否可安全释放本地空间。
	syncCheckSafeFreeUp: (relPath: string, fileId: string) => __TAURI_INVOKE<FreeUpCheckResult>("sync_check_safe_free_up", { relPath, fileId }),
	// 查询文件本地同步状态（供前端删除确认用）。
	// 返回 "folder" | "synced" | "placeholder" | "not_synced"
	syncCheckFileLocalStatus: (fileId: string) => __TAURI_INVOKE<FileLocalStatus>("sync_check_file_local_status", { fileId }),
	// 批量查询文件同步状态（供前端文件列表状态列展示用）。
	// 未挂载同步目录时回退到仅 DB 状态判断。
	// 数据库与文件系统严格分段：先在锁内读完基线与在途任务，再释放锁，
	// 把 stat 与 xattr（`is_placeholder_file`）放到阻塞线程池批量执行。
	// 旧实现整段持有全局 DB 锁做逐条文件 I/O，大目录会让引擎写库一起排队（D14）。
	syncBatchFileStatus: (fileIds: string[]) => __TAURI_INVOKE<{ [key in string]: FileLocalStatus }>("sync_batch_file_status", { fileIds }),
	// 将已同步文件替换为按需下载占位符。
	syncFreeUpSpace: (fileId: string, relPath: string, localPath: string, name: string, size: number) => __TAURI_INVOKE<null>("sync_free_up_space", { fileId, relPath, localPath, name, size }),
	// 批量释放多个文件的本地空间，逐项独立执行。
	// 单项失败（如并发改动、远端版本漂移）只记录原因并跳过，不中断整体释放；
	// 每项独立持有路径租约，互不阻塞。返回成功/跳过计数与释放总字节。
	// - `items` 由前端弹窗确认的可释放候选项清单
	syncFreeUpBatch: (items: FreeableItem[]) => __TAURI_INVOKE<FreeUpBatchResult>("sync_free_up_batch", { items }),
	// 枚举目录（含子树）下可释放空间的文件候选项。
	// 仅基于 DB 成功同步基线筛选 status=SYNCED 且非目录的记录，供前端弹窗预览；
	// 实际释放前由 `free_up_one` 逐项重新核验，避免预览与释放之间状态漂移造成误释放。
	// 路径匹配用精确前缀加路径分隔符边界，避免 `docs` 误匹配 `docs-backup` 这类同级异名目录。
	// - `folder_rel_path` 目录相对挂载根的路径，传空串表示从根枚举
	syncListFreeableInFolder: (folderRelPath: string) => __TAURI_INVOKE<FreeableItem[]>("sync_list_freeable_in_folder", { folderRelPath }),
	// 按需下载占位文件，并等待本次任务结算（「下载后立即打开」依赖这一点）。
	syncDownloadOnDemand: (fileId: string, destPath: string) => __TAURI_INVOKE<DownloadOnDemandOutcome>("sync_download_on_demand", { fileId, destPath }),
	// 批量按需下载：只入队并立即返回，进度由传输队列呈现。
	// 批量场景逐个等待下载完成会让界面卡住数分钟，且失败难以汇总；
	// 这里只投递任务，由后台按并发槽执行。
	syncDownloadEnqueue: (fileId: string, destPath: string) => __TAURI_INVOKE<DownloadOnDemandOutcome>("sync_download_enqueue", { fileId, destPath }),
	// 入队一次目录双端对齐。
	// 返回作业快照（含排队状态）；同一目录重复点击会复用活跃作业。
	syncFolderEnqueue: (folderId: string, relPath: string) => __TAURI_INVOKE<FolderSyncJob>("sync_folder_enqueue", { folderId, relPath }),
	// 列出目录同步作业（活跃在前，其后是最近的终态历史）。
	syncFolderJobs: () => __TAURI_INVOKE<FolderSyncJob[]>("sync_folder_jobs"),
	// 取消一个排队中或规划中的目录同步作业。
	syncFolderCancel: (jobId: number) => __TAURI_INVOKE<boolean>("sync_folder_cancel", { jobId }),
	// 重试全部失败任务：全部入队后立即返回汇总，不等待执行。
	syncRetryFailed: () => __TAURI_INVOKE<RetryAllSummary>("sync_retry_failed"),
	// 确认执行本轮被安全闸门拦下的大批量云端删除。
	// 只有在状态里看到 `blocked_cloud_deletes > 0` 时才应该调用；确认后引擎会立刻
	// 重新规划一次，这批删除才会真正下发。
	syncApproveMassDelete: () => __TAURI_INVOKE<null>("sync_approve_mass_delete"),
	// 清理一个已失效的本地占位文件。
	// 只在「本地仍是 0 字节占位符、且云端确实没有该路径」时才会删除；
	// 云端副本若已恢复，命令会拒绝并提示，避免把用户刚恢复的内容二次删掉。
	syncCleanupPlaceholder: (relPath: string) => __TAURI_INVOKE<boolean>("sync_cleanup_placeholder", { relPath }),
	// 获取完整同步状态快照。
	syncState: () => __TAURI_INVOKE<SyncGlobalState_Serialize>("sync_state"),
	// 查询目录下的同步项。
	syncItemsByFolder: (folderLocalPath: string) => __TAURI_INVOKE<SyncItem[]>("sync_items_by_folder", { folderLocalPath }),
	// 读取并校验当前持久化配置。
	configLoad: () => __TAURI_INVOKE<AppConfig>("config_load"),
	// 保存配置；挂载目录变化时停止旧运行时、清理缓存并重启，运行参数变化时原位重建引擎。
	configSave: (config: AppConfig) => __TAURI_INVOKE<null>("config_save", { config }),
	// 保存列表排序偏好。
	// 这是纯展示设置：只改排序字段，不触发引擎重建；单独成命令是为了避免
	// 每次点击表头都整份读改写配置，与设置页保存互相踩踏。
	configSaveSort: (field: SortField, order: SortOrder) => __TAURI_INVOKE<null>("config_save_sort", { field, order }),
	// 切换托盘图标显示：持久化到配置并立即生效（对齐开机自启开关的即时生效模式）。
	traySetVisible: (visible: boolean) => __TAURI_INVOKE<null>("tray_set_visible", { visible }),
	// 将当前配置序列化为可导入的 JSON 文本。
	configExportJson: () => __TAURI_INVOKE<string>("config_export_json"),
	// 解析并校验 JSON 配置，但不在此入口直接覆盖当前配置文件。
	configImportJson: (jsonStr: string) => __TAURI_INVOKE<AppConfig>("config_import_json", { jsonStr }),
	// 列出传输任务。
	transferListAll: () => __TAURI_INVOKE<TransferTask[]>("transfer_list_all"),
	// 检查活动传输。
	transferHasActive: () => __TAURI_INVOKE<boolean>("transfer_has_active"),
	// 清除已完成传输。
	transferClearCompleted: () => __TAURI_INVOKE<null>("transfer_clear_completed"),
	// 清除失败传输。
	transferClearFailed: () => __TAURI_INVOKE<null>("transfer_clear_failed"),
	// 清除已结束传输。
	transferClearFinished: () => __TAURI_INVOKE<null>("transfer_clear_finished"),
	// 重试传输任务。
	// 只做幂等入队并立即返回，执行由后台调度；重复点击返回 `AlreadyQueued`。
	transferRetry: (taskId: number) => __TAURI_INVOKE<RetryEnqueued>("transfer_retry", { taskId }),
	// 同名冲突决策：覆盖远端（远端旧版自动改名为云端副本保留，不丢数据）。
	transferOverwriteRemote: (taskId: number) => __TAURI_INVOKE<null>("transfer_overwrite_remote", { taskId }),
	// 同名冲突决策：保留两者（本地文件改名为本地副本后作为新文件上传，云端原文件不动）。
	transferKeepBoth: (taskId: number) => __TAURI_INVOKE<null>("transfer_keep_both", { taskId }),
	// 取消「需要重新检查」或失败的传输任务。
	transferCancel: (taskId: number) => __TAURI_INVOKE<null>("transfer_cancel", { taskId }),
	// 批量取消全部失败任务，返回实际取消数量。
	// 队列里几百条失败任务只有「重试全部」时，用户无法清走已知脏任务；
	// 取消保留历史行但释放路径占用，让被隔离的同步动作继续执行。
	transferCancelAllFailed: () => __TAURI_INVOKE<number>("transfer_cancel_all_failed"),
	// 在系统文件管理器中打开路径。
	openInFinder: () => __TAURI_INVOKE<boolean>("open_in_finder"),
	// 使用系统默认应用打开同步目录内的本地文件或目录。
	openLocalItem: (relPath: string) => __TAURI_INVOKE<boolean>("open_local_item", { relPath }),
	// 在系统文件管理器中显示同步目录内的本地文件或目录。
	// 此操作允许显示占位文件，但不会把占位文件作为内容交给默认应用。
	revealLocalItem: (relPath: string) => __TAURI_INVOKE<boolean>("reveal_local_item", { relPath }),
	// 使用系统默认浏览器打开合法的 HTTP/HTTPS 外部链接。
	openExternalUrl: (url: string) => __TAURI_INVOKE<boolean>("open_external_url", { url }),
	// 查询 Linux 按需云盘当前是否真实挂载。
	// 前端不能只看配置开关：引擎启动和首次索引是异步的，FUSE 也可能因系统能力问题
	// 挂载失败；此命令让“打开云盘”入口只在真正可用时出现。
	virtualDriveStatus: () => __TAURI_INVOKE<VirtualDriveStatus>("virtual_drive_status"),
	// 检查开机自启。
	launchAtLoginIsEnabled: () => __TAURI_INVOKE<boolean>("launch_at_login_is_enabled"),
	// 设置开机自启。
	launchAtLoginSetEnabled: (enabled: boolean) => __TAURI_INVOKE<boolean>("launch_at_login_set_enabled", { enabled }),
	// 查询托盘图标当前实际可见性（运行时真实状态，而非配置文件里的目标值）。
	trayIsVisible: () => __TAURI_INVOKE<boolean>("tray_is_visible"),
	// 当前发行渠道是否提供可验证的自动更新产物。
	// macOS 沿用已有的签名更新渠道。Linux 必须同时满足：
	// - 构建时显式设置 `PETALLINK_UPDATE_CHANNEL=appimage`；
	// - 当前确实由 AppImage runtime 启动；
	// - AppImage 所在目录允许当前用户原子替换该文件。
	// 因此 AUR/系统包和普通源码构建即使人为设置 `APPIMAGE` 也不会误走应用内更新，
	// 放在只读目录中的 AppImage 则交由用户或包管理器更新。
	updaterIsSupported: () => __TAURI_INVOKE<boolean>("updater_is_supported"),
	// 收束同步运行时并以前台模式重启应用。
	appRelaunch: () => __TAURI_INVOKE<null>("app_relaunch"),
	// 清空应用缓存。
	appClearCache: () => __TAURI_INVOKE<null>("app_clear_cache"),
	// 读取最近日志；`since_ms` 只返回不早于该时间戳的记录（日志页时间筛选）。
	logsList: (sinceMs: number | null) => __TAURI_INVOKE<LogRecord[]>("logs_list", { sinceMs }),
	// 导出完整日志。
	// 逐文件流式写入（临时文件 + 原子替换），不再把全部日志拼成一个字符串进内存：
	// 日志量大时旧实现会一次性占用与日志总量相等（甚至翻倍）的内存（D24）。
	logsExport: (path: string) => __TAURI_INVOKE<null>("logs_export", { path }),
	// 清空内存日志缓冲区；磁盘滚动日志由保留策略单独管理。
	logsClear: () => __TAURI_INVOKE<null>("logs_clear"),
	// 获取应用版本。
	appGetVersion: () => __TAURI_INVOKE<string>("app_get_version"),
	// 获取当前构建的展示名（dev 包带 `-Dev` 后缀）。
	appDisplayName: () => __TAURI_INVOKE<string>("app_display_name"),
	// 获取应用完整标题（窗口标题栏与登录页共用同一文案）。
	appFullTitle: () => __TAURI_INVOKE<string>("app_full_title"),
};

// 事件
export const events = {
	authLoginLaunched: makeEvent<AuthLoginLaunchedEvent>("auth_login_launched"),
	folderContentChanged: makeEvent<FolderContentChangedEvent>("folder_content_changed"),
	folderSyncJobsChanged: makeEvent<FolderSyncJobsChangedEvent>("folder_sync_jobs_changed"),
	navigateSettings: makeEvent<NavigateSettingsEvent>("navigate_settings"),
	syncState: makeEvent<SyncStateEvent_Deserialize>("sync_state"),
	transferSettled: makeEvent<TransferSettledEvent>("transfer_settled"),
	transferUpdate: makeEvent<TransferUpdateEvent>("transfer_update"),
	uploadFailed: makeEvent<UploadFailedEvent>("upload_failed"),
	virtualDriveStatus: makeEvent<VirtualDriveStatusEvent>("virtual_drive_status"),
};

// 常量
export const DELETE_TRACE_ERROR_PREFIX = "TRACE_FAILED:" as const;

export const FOLDER_SYNC_JOB_STATE = {"CANCELED":4,"COMPLETED":2,"FAILED":3,"PLANNING":1,"QUEUED":0} as const;

export const SYNC_USER_MESSAGE_RULES = [{"message":"云端文件已更新。为避免覆盖，请同步索引后重试。","patterns":["远端文件已在规划后变化","云端文件版本已变化"]},{"message":"文件正在编辑，保存并关闭后会自动继续。","patterns":["用户正在编辑","文件正在编辑"]},{"message":"文件仍在变化，稳定后会自动继续。","patterns":["文件尚不稳定","文件仍在变化"]},{"message":"本地文件已发生变化，请重新检查并重试。","patterns":["本地上传源已变化","本地上传源在执行前发生变化","本地源已变化","下载目标已出现本地内容","更新下载目标已变化","更新下载目标已不存在"]},{"message":"文件同步信息不完整，请同步索引后重试。","patterns":["缺少 fileId","缺少真实 fileId","缺少 parentId","缺少 operation","operation 与 direction 不一致","缺少云端版本","缺少云端版本快照"]},{"message":"续传信息已失效，请重新开始上传。","patterns":["session_url","上传断点","安全重放"]},{"message":"没有找到可用于核对的同步记录，暂时无法释放空间。","patterns":["找不到与路径匹配的成功同步基线"]},{"message":"本地文件已更改，无法释放空间。","patterns":["本地内容与最后成功同步基线不一致"]},{"message":"云端文件信息已变化，请同步索引后重试。","patterns":["可信云树中不存在同一 fileId"]},{"message":"云端文件已变化，无法释放空间。","patterns":["远端副本不存在、已回收、大小或版本与成功基线不一致"]},{"message":"检查期间本地文件发生变化，无法释放空间。","patterns":["远端核验期间本地文件已变化"]},{"message":"云端文件仍在更新，请稍后再试。","patterns":["云端索引尚未追平"]},{"message":"文件状态已变化，请同步索引后重试。","patterns":["释放租约已失效"]},{"message":"文件状态已变化，请重新检查并重试。","patterns":["重新规划"]},{"message":"正在确认同步结果，请稍后查看。","patterns":["远端核验"]}] as const;

export const TRANSFER_DIR = {"DELETE":2,"DOWNLOAD":1,"DOWNLOAD_UPDATE":3,"UPLOAD":0} as const;

export const TRANSFER_ERROR_KIND = {"AUTH":2,"LOCAL_CHANGED":10,"NAME_CONFLICT":12,"NETWORK":0,"PERMISSION":6,"QUOTA":5,"RATE_LIMIT":3,"REMOTE_AMBIGUOUS":9,"SERVER":4,"SESSION_EXPIRED":8,"TIMEOUT":1,"UNKNOWN":11,"VALIDATION":7} as const;

export const TRANSFER_OPERATION = {"CREATE":0,"CREATE_FOLDER":7,"DELETE":4,"DOWNLOAD":2,"DOWNLOAD_UPDATE":3,"MOVE":5,"RENAME":6,"UPDATE":1} as const;

export const TRANSFER_STATE = {"BACKING_OFF":3,"CANCELED":8,"COMPLETED":6,"FAILED":7,"PENDING":0,"RESTART_REQUIRED":5,"RUNNING":1,"VERIFYING_REMOTE":4,"WAITING_FOR_NETWORK":2} as const;

// 类型
// 应用配置。
// 默认值：concurrency=6, pollIntervalSec=60（0=关闭自动刷新）, debounceSec=3。
export type AppConfig = {
	// OAuth 回调 URI（必须与 AGC 后台一致）
	oauth_redirect_uri?: string,
	// OAuth 回调端口
	oauth_callback_port?: number,
	// 同步引擎使用的本地物理目录（可能含 ~ 前缀）。
	// Linux 上这是应用管理的隐藏 backing，不是用户选择或日常打开的目录；
	// 用户选择的唯一目录保存在 [`Self::virtual_mount_dir`]。
	mount_dir?: string,
	// 用户是否已显式配置过挂载目录（首次同步引导用，F-MOUNT-13）。
	// 区分"默认值"与"用户已确认"，避免未选目录就自动同步覆盖本地已有内容。
	mount_configured?: boolean,
	// 是否启用 Linux FUSE 按需云盘。
	// Linux 已配置目录时该值必须为 true；false 只用于“尚未配置”的初始态。
	virtual_drive_enabled?: boolean,
	// 用户可见的 FUSE 挂载目录；`mount_dir` 仍作为物理 backing 目录。
	virtual_mount_dir?: string,
	// 并发传输数，范围 1-20（Q1 决策：默认 6）
	concurrency?: number,
	// 云端定时刷新间隔（秒）。0 = 关闭自动刷新；开启时最小 60 秒。默认 900（15 分钟）。
	// 每次到期全量 BFS 重拉云端树，使云端的新增/修改/删除自动同步到本地。
	poll_interval_sec?: number,
	// 变更 debounce 时长，默认 3 秒（F-MOUNT-09）
	debounce_sec?: number,
	// 跳过文件列表（通配符）
	skip_patterns?: string[],
	// 排序字段
	sort_field?: SortField,
	// 排序方向
	sort_order?: SortOrder,
	// 是否显示系统托盘图标。默认显示。
	// 关闭后后台同步无托盘入口，此时关闭主窗口或退出应用会直接真退出。
	show_tray_icon?: boolean,
};

// 所有自定义异常基类。序列化为前端可解析的扁平结构。
// 自定义 Serialize 把字段提到顶层（`kind`/`code`/`message`/`status_code`/`error_code`），
// `message` 始终是字符串。这样前端 `AppError.message: string` 直接可读，
// 避免默认 tagged-enum 序列化把 payload 嵌套进 `message` 导致渲染成 `[object Object]`。
// `AppError` 自身包含仅供后端恢复和重试使用的字段，并通过手写 `Serialize`
// 输出为 `IpcError` 结构；Specta 使用同一结构生成前端类型。
export type AppError = {
	// 错误类别。
	kind: IpcErrorKind,
	// 类别内错误码。
	code: string | null,
	// 面向用户的错误消息。
	message: string,
	// Drive API HTTP 状态码。
	status_code: number | null,
	// 华为 Drive API 错误码。
	error_code: string | null,
};

// 登录界面已拉起，携带实际使用的形态。
// 前端据此决定是否保留底部抽屉：后端可能因为 child webview 不可用而退回独立窗口
// 或系统浏览器，这时抽屉必须收起来，否则会留下一块空白。
export type AuthLoginLaunchedEvent = {
	// 形态：`sheet` / `window` / `browser`。
	mode: string,
};

// 前端恢复认证页面所需的登录、凭据与回调端口快照。
export type AuthState = {
	logged_in: boolean,
	secret_configured: boolean,
	callback_port: number,
};

// 按需下载的处理结果。
// 「已加入传输队列」不是失败：文件会在队列里完成落盘，UI 必须给出可继续的提示，
// 而不是把成功入队渲染成红色的「同步失败」。
export type DownloadOnDemandOutcome =
// 本次已下载完成，可以立即打开。
"completed" |
// 已加入传输队列，完成后自动落盘。
"queued";

// Drive 配额信息。对齐 dart `DriveAbout`。
export type DriveAbout = {
	user_capacity: number,
	used_space: number,
	user_display_name: string | null,
};

// Drive 文件 DTO（对应华为云盘 File 资源）。
// 对齐 dart `DriveFile`。
export type DriveFile = {
	id: string,
	name: string,
	category: FileCategory,
	size: number,
	parent_folder: string[] | null,
	description: string | null,
	created_time: string | null,
	edited_time: string | null,
	mime_type: string | null,
	// 云端内容 hash（md5/sha256，字段名兼容多种）。
	// 若华为返回则为内容指纹，用于精确变更检测；为 null 时降级用 editedTime。
	content_hash: string | null,
	thumbnail_link: string | null,
};

// 失败项详情（前端失败项弹窗用）
export type FailedItem = {
	// 相对路径（取自 sync_items.local_path）
	relative_path: string,
	// 错误信息
	error_message: string | null,
	// 该路径对应的传输任务 ID；没有对应任务（纯基线失败）时为 None。
	// 前端据此提供「单条重试」，无需再去传输队列里翻。
	task_id: number | null,
};

// 文件分类
export type FileCategory = "folder" | "audio" | "video" | "image" | "document" | "package" | "archive" | "executable" | "none";

// 文件列表结果。对齐 dart `FileListResult`。
export type FileListResult = {
	files: DriveFile[],
	next_cursor: string | null,
};

// 文件在本地挂载目录中的可观察同步状态。
// 前四个取值描述「本地事实」，后三个来自 `transfer_queue` 的在途事实：
// 文件列表必须能直接回答「谁在传、谁在确认、谁失败了」，而不是让用户去传输队列里猜。
export type FileLocalStatus =
// 本地项是目录。
"folder" |
// 本地真实文件与成功同步基线一致。
"synced" |
// 本地项是按需下载占位文件。
"placeholder" |
// 本地不存在可信同步文件。
"not_synced" |
// 该路径有排队/执行/退避中的传输任务。
"syncing" |
// 上传已完成，正在确认云端写入结果。
"verifying" |
// 该路径最近一次同步失败，等待重试。
"failed";

// 云盘目录内容已变化，前端应重新加载当前目录。
export type FolderContentChangedEvent = null;

// 一次「双端对齐」作业（对应 `folder_sync_jobs` 表一行）。
export type FolderSyncJob = {
	// 自增主键。
	id: number,
	// 云端目录 ID。
	folder_id: string,
	// 挂载根内的相对路径。
	rel_path: string,
	// 作业状态（见 `job_state`）。
	state: number,
	// 本轮需要处理的文件总数；规划完成前为 0。
	planned_total: number,
	// 已投递到传输队列的文件数。
	enqueued: number,
	// 失败原因。
	error_message: string | null,
	// 入队时间（毫秒）。
	created_at: number,
	// 开始规划时间（毫秒）。
	started_at: number | null,
	// 结束时间（毫秒）。
	finished_at: number | null,
	// 乐观并发版本。
	state_revision: number,
};

// 目录同步作业发生变化，前端应重新加载作业列表。
export type FolderSyncJobsChangedEvent = null;

// 批量释放空间结果统计。
// 与前端 `FreeUpBatchResult` interface 的合同为 camelCase，缺失时前端读不到计数。
export type FreeUpBatchResult = {
	// 成功释放的文件数
	freedCount: number,
	// 因不满足条件被跳过的文件数
	skippedCount: number,
	// 成功释放的总字节数
	freedBytes: number,
	// 被跳过项的错误原因（与跳过项一一对应，便于前端提示）
	errors: string[],
};

// 释放空间安全校验结果
export type FreeUpCheckResult =
// 可以安全释放
"safe" |
// 云端不存在（释放后无法找回）
"not_in_cloud" |
// 本地尚未同步到云端（有未上传修改）
"not_synced";

// 可释放空间候选项（基于 DB 基线枚举，实际释放前再逐项安全核验）。
// 与前端 `FreeableItem` interface 的合同为 camelCase，序列化/反序列化必须保持一致。
export type FreeableItem = {
	// 云端文件 ID
	fileId: string,
	// 相对挂载目录的路径
	relPath: string,
	// 文件名
	name: string,
	// 本地已下载字节数
	size: number,
};

// 拖拽导入的单个失败项。
export type ImportFailure = {
	// 导入源的原始路径。
	source: string,
	// 用户可读的中文失败原因。
	reason: string,
};

// 拖拽导入结果汇总。
export type ImportFilesResult = {
	// 成功复制的文件数（目录按其中文件逐个计）。
	imported: number,
	// 命中跳过规则或为符号链接而被跳过的条目数。
	skipped: number,
	// 失败的导入源及原因（含同名冲突拒绝覆盖）。
	failures: ImportFailure[],
};

// 失效占位文件明细（前端提示用）。
export type InvalidPlaceholder = {
	// 相对挂载根的路径。
	relative_path: string,
};

// `AppError` 暴露给前端的稳定扁平结构。
export type IpcError = {
	// 错误类别。
	kind: IpcErrorKind,
	// 类别内错误码。
	code: string | null,
	// 面向用户的错误消息。
	message: string,
	// Drive API HTTP 状态码。
	status_code: number | null,
	// 华为 Drive API 错误码。
	error_code: string | null,
};

// 前端用于分流展示逻辑的错误类别。
export type IpcErrorKind =
// OAuth 流程错误。
"Auth" |
// Token 状态或刷新错误。
"Token" |
// Drive API 请求错误。
"DriveApi" |
// 配置读取或校验错误。
"Config" |
// 云盘剩余配额不足。
"QuotaExceeded" |
// 无法归入其他类别的错误。
"Generic";

// 日志级别（前端展示用，对齐 dart logging 的 Level）
export type LogLevel = "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE";

// 单条日志记录（对齐 dart `LogRecord`）
export type LogRecord = {
	// 级别
	level: LogLevel,
	// logger 名称（模块名）
	logger_name: string,
	// 消息内容
	message: string,
	// 时间戳（毫秒，epoch）
	time_ms: number,
};

// 原生菜单请求打开设置页。
export type NavigateSettingsEvent = null;

// 一键重试全部失败项的汇总。
export type RetryAllSummary = {
	// 本次新入队的任务数。
	queued: number,
	// 已在队列中而跳过的任务数。
	already_queued: number,
	// 不可重试而跳过（同名冲突、未知操作类型）。
	skipped: number,
	// 校验失败的任务数（源文件已变化等）。
	rejected: number,
};

// 单个重试请求的入队结果。
// `Queued`/`AlreadyQueued` 都属于成功：前者表示本次请求把任务推进到 Pending，
// 后者表示任务本来就在队列里（幂等重复点击），前端无需区分即可继续。
export type RetryEnqueued =
// 本次请求已把任务推进到 Pending。
({ Queued: {
	// 被接受的任务 ID。
	task_id: number,
} }) & { AlreadyQueued?: never } |
// 任务本来就在队列中（幂等），不视为错误。
({ AlreadyQueued: {
	// 已处于队列中的任务 ID。
	task_id: number,
} }) & { Queued?: never };

// 底部抽屉在主窗口内的矩形（整数逻辑像素，原点为窗口内容区左上角）。
// 前端已经按 CSS 像素取整，这里用整数避免浮点边界误差，也让生成的 TS 类型是 `number`。
export type SheetBounds = {
	// 左边距。
	x: number,
	// 上边距。
	y: number,
	// 宽度。
	width: number,
	// 高度。
	height: number,
	// 整卡高度（含标题区），用于换算滑入行程。
	// 前端整卡从「卡片高度」的下方滑入，原生视图必须走同样长的距离，
	// 否则动画中登录页会短暂压住标题区。
	sheetHeight: number,
	// 窗口 CSS 视口高度（前端 `window.innerHeight`）。
	// 原生子视图以外框顶边为原点定位，而 CSS 视口从内容区开始，两者相差一个标题栏。
	// 这个偏移量只能由「平台报告的内容区高度 − 真实视口高度」求得：macOS 上平台把
	// 标题栏算进了内容区（`inner_size` == 外框尺寸），单看平台数值无法区分。
	viewportHeight: number,
};

// 同步状态展示排序字段
export type SortField = "name" | "size" | "modifiedTime";

// 列表排序方向
export type SortOrder = "ascending" | "descending";

// 同步全局状态（对齐 dart SyncGlobalState，供 UI 透传）
export type SyncGlobalState = SyncGlobalState_Serialize | SyncGlobalState_Deserialize;

// 同步全局状态（对齐 dart SyncGlobalState，供 UI 透传）
export type SyncGlobalState_Deserialize = {
	// 权威快照的进程内单调版本。
	revision: number,
	total: number,
	completed: number,
	uploading: number,
	downloading: number,
	// 因网络不可用而等待恢复的传输任务数（不属于永久失败）。
	waiting_network: number,
	failed: number,
	// 传输队列中永久失败的历史任务数，与当前同步失败分开统计。
	transfer_failed: number,
	// 失败项详情（供 SyncStatusBar 失败项弹窗，最多 20 条）
	failed_items: FailedItem[],
	conflict: number,
	// 被暂停编辑的文件数（F-MOUNT-11）
	editing: number,
	// 引擎是否正在运行
	is_running: boolean,
	// 上次同步时间（毫秒 epoch）
	last_sync_time: number | null,
	// 是否正在索引云端目录
	is_indexing: boolean,
	// 已扫描的文件夹数（索引用）
	indexing_scanned_folders: number,
	// 已发现的文件总数（索引用）
	indexing_discovered_items: number,
	// 是否有目录结构变更（触发前端目录重拉）
	content_changed: boolean,
	// 本地已失效的占位文件（云端副本已不存在）。
	// 这些路径本地只剩 0 字节占位符，云端内容已被删除：既不能上传成空文件，
	// 也不应静默清理。UI 展示出来让用户决定「恢复云端副本」还是「清理本地残留」。
	invalid_placeholders: InvalidPlaceholder[],
	// 本轮被安全闸门拦下的云端删除条目数。
	// 大于 0 表示存在「疑似误删」的批量删除（例如挂载目录被清空、卷未挂载、
	// 或一次要删掉超过阈值的文件），引擎已拒绝执行，必须由用户显式确认。
	blocked_cloud_deletes: number,
	// 本轮因同路径仍有活动任务而被暂缓的**结构性**同步动作数。
	// 只统计删除/建目录/移动/占位等结构动作（文件传输的暂缓由传输队列自身展示）。
	// 这些动作不是失败：等占用路径的传输任务结算（或被取消）后，下一轮会自动重试。
	// 大于 0 时 UI 必须提示「N 项同步等待任务结束」，避免用户把暂缓当成按钮失灵。
	deferred_actions: number,
	// 当前同步阶段（供前端状态条精确显示）。None = 空闲。
	// 值由本模块 `SYNC_PHASE_*` 常量定义；运行中不得为空。
	sync_phase?: string | null,
};

// 同步全局状态（对齐 dart SyncGlobalState，供 UI 透传）
export type SyncGlobalState_Serialize = {
	// 权威快照的进程内单调版本。
	revision: number,
	total: number,
	completed: number,
	uploading: number,
	downloading: number,
	// 因网络不可用而等待恢复的传输任务数（不属于永久失败）。
	waiting_network: number,
	failed: number,
	// 传输队列中永久失败的历史任务数，与当前同步失败分开统计。
	transfer_failed: number,
	// 失败项详情（供 SyncStatusBar 失败项弹窗，最多 20 条）
	failed_items: FailedItem[],
	conflict: number,
	// 被暂停编辑的文件数（F-MOUNT-11）
	editing: number,
	// 引擎是否正在运行
	is_running: boolean,
	// 上次同步时间（毫秒 epoch）
	last_sync_time: number | null,
	// 是否正在索引云端目录
	is_indexing: boolean,
	// 已扫描的文件夹数（索引用）
	indexing_scanned_folders: number,
	// 已发现的文件总数（索引用）
	indexing_discovered_items: number,
	// 是否有目录结构变更（触发前端目录重拉）
	content_changed: boolean,
	// 本地已失效的占位文件（云端副本已不存在）。
	// 这些路径本地只剩 0 字节占位符，云端内容已被删除：既不能上传成空文件，
	// 也不应静默清理。UI 展示出来让用户决定「恢复云端副本」还是「清理本地残留」。
	invalid_placeholders: InvalidPlaceholder[],
	// 本轮被安全闸门拦下的云端删除条目数。
	// 大于 0 表示存在「疑似误删」的批量删除（例如挂载目录被清空、卷未挂载、
	// 或一次要删掉超过阈值的文件），引擎已拒绝执行，必须由用户显式确认。
	blocked_cloud_deletes: number,
	// 本轮因同路径仍有活动任务而被暂缓的**结构性**同步动作数。
	// 只统计删除/建目录/移动/占位等结构动作（文件传输的暂缓由传输队列自身展示）。
	// 这些动作不是失败：等占用路径的传输任务结算（或被取消）后，下一轮会自动重试。
	// 大于 0 时 UI 必须提示「N 项同步等待任务结束」，避免用户把暂缓当成按钮失灵。
	deferred_actions: number,
	// 当前同步阶段（供前端状态条精确显示）。None = 空闲。
	// 值由本模块 `SYNC_PHASE_*` 常量定义；运行中不得为空。
	sync_phase?: string | null,
};

// 同步状态项实体（对应 sync_items 表一行）。
// 对齐 dart `SyncItemEntity`。
export type SyncItem = {
	// 云端文件 ID（主键之一）
	file_id: string,
	// 相对挂载根的规范 UTF-8 路径（主键之二）
	local_path: string,
	// 父目录 fileId
	parent_folder_id: string | null,
	// 文件名
	name: string,
	// 是否文件夹
	is_folder: boolean,
	// 云端大小（字节）
	size: number,
	// 本地大小（字节，v3，变更检测用）
	local_size: number | null,
	// 本地 SHA256
	sha256: string | null,
	// 本地 mtime（毫秒）
	local_mtime: number | null,
	// 云端 editedTime（毫秒）
	cloud_edited_time: number | null,
	// 最后成功同步时间（毫秒）
	last_sync_time: number | null,
	// 同步状态（见 sync_status 常量）
	status: number,
	// 失败/冲突原因
	error_message: string | null,
};

// 完整同步状态快照事件。
export type SyncStateEvent = SyncStateEvent_Serialize | SyncStateEvent_Deserialize;

// 完整同步状态快照事件。
export type SyncStateEvent_Deserialize =
// 当前完整同步状态。
SyncGlobalState_Deserialize;

// 完整同步状态快照事件。
export type SyncStateEvent_Serialize =
// 当前完整同步状态。
SyncGlobalState_Serialize;

// OAuth Token 对（需求 F-AUTH-03）。
// access_token + refresh_token + 过期时间，加密持久化到本地文件（机器码绑定）。
export type TokenPair = {
	access_token: string,
	refresh_token: string,
	// access_token 过期时间（**毫秒**时间戳，对齐 dart）
	expires_at: number,
	token_type?: string,
	scope: string | null,
};

// 单个传输任务成功结算事件。
// 前端据此主动重载当前目录，不必再等云轮询周期（默认 60 秒）才发现新上传的文件。
export type TransferSettledEvent = {
	// 相对挂载目录的路径。
	relative_path: string,
};

// 传输任务实体（对应 transfer_queue 表一行）。
// 对齐 dart `TransferTaskEntity`。
export type TransferTask = {
	// 自增主键
	id: number,
	// 上传/下载（见 transfer_direction 常量）
	direction: number,
	// 关联的 SyncItem fileId（可空，手动传输无对应项）
	file_id: string | null,
	// 本地路径（可空）
	local_path: string | null,
	// 文件名
	name: string,
	// 总大小（字节）
	total_size: number,
	// 已传输（字节）
	transferred: number,
	// 传输状态（见 transfer_state 常量）
	state: number,
	// 失败原因
	error_message: string | null,
	// 入队时间（毫秒）
	created_at: number,
	// 完成时间（毫秒）
	finished_at: number | null,
	// 华为 resume 上传会话标识（v2）
	server_id: string | null,
	// 华为 uploadId（v2）
	upload_id: string | null,
	// 已上传字节偏移（断点续传恢复点，v2）
	resume_offset: number,
	// 华为 resume 上传 Location 头返回的会话 URL（v4，断点续传必需的唯一 token）。
	// 新 API 不再在 body 返回 serverId/uploadId，分片 PUT 必须直接用此 URL。
	session_url: string | null,
	// 相对挂载根的规范 UTF-8 路径（绝不替代 absolute local_path）。
	relative_path: string | null,
	// 规划时的云端父目录 fileId。
	parent_file_id: string | null,
	// 持久化操作类型（见 `TransferOperation`）。
	operation: number | null,
	// 入队时本地源 mtime 快照。
	source_mtime: number | null,
	// 入队时本地源大小快照。
	source_size: number | null,
	// 规划时观察到的云端 editedTime。
	expected_cloud_edited_time: number | null,
	// 已消耗的持久化尝试次数。
	attempt_count: number,
	// 远端核验专用尝试次数，独立于全局重试预算 `attempt_count`，避免核验循环虚增预算。
	verify_attempt_count: number,
	// 下一次允许重试的时间戳。
	next_retry_at: number | null,
	// 结构化错误类型（见 `TransferErrorKind`）。
	error_kind: number | null,
	// 远端结果复核确认的资源 fileId。
	remote_result_file_id: string | null,
	// 乐观并发状态版本。
	state_revision: number,
};

// 传输队列已变化，前端应重新加载队列。
export type TransferUpdateEvent = null;

// 后台自动上传失败事件。
export type UploadFailedEvent = {
	// 相对挂载目录的文件路径。
	rel_path: string,
	// 上传失败的文件名。
	name: string,
	// 面向用户的失败原因。
	error: string,
};

// 华为账号信息 DTO（合并自多个端点响应）。
// 对齐 dart `UserInfo`。
export type UserInfo = {
	sub: string | null,
	open_id: string | null,
	union_id: string | null,
	display_name: string | null,
	name: string | null,
	nickname: string | null,
	email: string | null,
	mobile: string | null,
	avatar_url: string | null,
	// displayName 是否为匿名账号（displayNameFlag=1）
	is_anonymized?: boolean,
};

// Linux 按需云盘的真实运行状态，而不是仅反映配置开关。
export type VirtualDriveStatus = {
	// 配置中是否启用了按需云盘。
	enabled: boolean,
	// FUSE 会话是否已经成功挂载。
	mounted: boolean,
	// 当前真实挂载目录；尚未挂载时为空。
	mount_dir: string | null,
	// 最近一次挂载失败原因。
	error: string | null,
};

// 按需云盘挂载成功或失败后推送真实状态。
export type VirtualDriveStatusEvent = VirtualDriveStatus;

// Tauri Specta 事件运行时
type EventEmit<T> = [T] extends [null] ? () => Promise<void> : (payload: T) => Promise<void>;

function makeEvent<T>(name: string, serialize?: (payload: T) => unknown, deserialize?: (payload: any) => T) {
    const mapEvent = (cb: __TAURI_EVENT.EventCallback<T>) => (event: __TAURI_EVENT.Event<any>) => cb({ ...event, payload: deserialize ? deserialize(event.payload) : event.payload });
    const mapPayload = (payload: T) => serialize ? serialize(payload) : payload;

    const base = {
        listen: (cb: __TAURI_EVENT.EventCallback<T>) => __TAURI_EVENT.listen(name, mapEvent(cb)),
        once: (cb: __TAURI_EVENT.EventCallback<T>) => __TAURI_EVENT.once(name, mapEvent(cb)),
        emit: ((payload: T) => __TAURI_EVENT.emit(name, mapPayload(payload)) as unknown) as EventEmit<T>
    };

    const fn = (target: import("@tauri-apps/api/webview").Webview | import("@tauri-apps/api/window").Window) => ({
        listen: (cb: __TAURI_EVENT.EventCallback<T>) => target.listen(name, mapEvent(cb)),
        once: (cb: __TAURI_EVENT.EventCallback<T>) => target.once(name, mapEvent(cb)),
        emit: ((payload: T) => target.emit(name, mapPayload(payload)) as unknown) as EventEmit<T>
    });

    return Object.assign(fn, base);
}
