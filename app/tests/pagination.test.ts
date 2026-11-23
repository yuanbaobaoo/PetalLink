// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from "vitest";
import { listFiles, searchFiles } from "@/api/drive";
import { commands } from "@/api/generated";
import type { DriveFile } from "@/api/generated";

vi.mock("@/api/generated", async (importOriginal) => {
  // 保留真实常量与类型，只替换两个分页命令。
  const actual = await importOriginal<typeof import("@/api/generated")>();
  return {
    ...actual,
    commands: {
      ...actual.commands,
      driveListAll: vi.fn(),
      driveSearchAll: vi.fn(),
      driveList: vi.fn(),
      driveSearch: vi.fn(),
    },
  };
});

/**
 * 构造一个满足列表展示需要的最小文件对象。
 */
function file(id: string, name: string, folder = false): DriveFile {
  return {
    id,
    name,
    category: folder ? "folder" : "video",
    size: 1,
    parent_folder: null,
    description: null,
    created_time: null,
    edited_time: null,
    mime_type: null,
    content_hash: null,
    thumbnail_link: null,
  } as DriveFile;
}

describe("目录与搜索分页", () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  it("目录列表取全量而不是第一页，并保持目录优先", async () => {
    // 超过单页上限（100）的完整结果集。
    const all = [
      ...Array.from({ length: 120 }, (_, index) => file(`f-${index}`, `file-${index}.mp4`)),
      file("dir-1", "文件夹A", true),
    ];
    vi.mocked(commands.driveListAll).mockResolvedValue(all);

    const listed = await listFiles("parent-1");

    expect(commands.driveListAll).toHaveBeenCalledWith("parent-1");
    // 关键回归：绝不能再走只取第一页的 driveList。
    expect(commands.driveList).not.toHaveBeenCalled();
    expect(listed).toHaveLength(121);
    expect(listed[0].id).toBe("dir-1");
  });

  it("搜索结果同样取全量", async () => {
    vi.mocked(commands.driveSearchAll).mockResolvedValue([
      file("f-1", "match-1.mp4"),
      file("dir-2", "匹配目录", true),
    ]);

    const matched = await searchFiles("match", "parent-1");

    expect(commands.driveSearchAll).toHaveBeenCalledWith("match", "parent-1");
    expect(commands.driveSearch).not.toHaveBeenCalled();
    expect(matched.map((item) => item.id)).toEqual(["dir-2", "f-1"]);
  });
});
