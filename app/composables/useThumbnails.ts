import { ref } from "vue";
import type { Ref } from "vue";
import type { DriveFile } from "@/api/generated";
import * as driveApi from "@/api/drive";

const THUMBNAIL_BATCH_SIZE = 6;

/**
 * 当前目录的缩略图缓存与分批加载。
 * 缓存随目录清空（clear()），在途批次由代次守卫作废。
 */
export function useThumbnails(sortedFiles: Ref<DriveFile[]>) {
  const thumbUrls = ref<Record<string, string>>({});
  // 加载代次：新批次启动时使在途批次的结果作废。
  let generation = 0;

  /**
   * 判断文件是否为可显示缩略图的类型（图片/视频）。
   * 空 MIME 按不支持缩略图处理。
   *
   * @param f - 文件对象
   */
  function isThumbnailType(f: DriveFile): boolean {
    const mime = f.mime_type ?? "";
    return mime.startsWith("image/") || mime.startsWith("video/");
  }

  /**
   * 获取文件的缩略图 URL（未加载时为空串）。
   *
   * @param f - 文件对象
   */
  function thumbUrl(f: DriveFile): string {
    return thumbUrls.value[f.id] ?? "";
  }

  /**
   * 清空缓存并使在途批次作废（目录切换时调用）。
   */
  function clear(): void {
    generation += 1;
    thumbUrls.value = {};
  }

  /**
   * 预加载当前列表中所有文件的缩略图
   */
  async function loadThumbs(): Promise<void> {
    const current = ++generation;
    // 当前目录内尚未缓存的图片和视频文件
    const targets = sortedFiles.value.filter(
      (file) => isThumbnailType(file) && !thumbUrls.value[file.id],
    );
    // 当前批次起始下标
    for (let index = 0; index < targets.length; index += THUMBNAIL_BATCH_SIZE) {
      // 目录已切换：在途批次结果属于旧目录，直接丢弃。
      if (current !== generation) return;
      // 当前限流批次
      const batch = targets.slice(index, index + THUMBNAIL_BATCH_SIZE);
      // 当前批次的缩略图结果
      const loaded = await Promise.all(batch.map(async (file) => ({
        fileId: file.id,
        url: await driveApi.getThumbnail(file.id),
      })));
      if (current !== generation) return;
      // 合并后的缩略图缓存
      const nextUrls = { ...thumbUrls.value };
      for (const item of loaded) {
        if (item.url) nextUrls[item.fileId] = item.url;
      }
      thumbUrls.value = nextUrls;
    }
  }

  return { thumbUrls, isThumbnailType, thumbUrl, clear, loadThumbs };
}
