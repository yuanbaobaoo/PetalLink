import { describe, expect, it, vi } from "vitest";
import type { ImportFilesResult } from "@/api/drive";
import { runDragImport, type DragUploadPort } from "@/composables/useDragUpload";

/**
 * 构造一次拖拽导入的后端汇总结果。
 */
function importResult(overrides: Partial<ImportFilesResult> = {}): ImportFilesResult {
  return {
    imported: 0,
    skipped: 0,
    failures: [],
    ...overrides,
  };
}

/**
 * 创建可观测调用顺序与通知内容的拖拽导入端口。
 */
function port(options: {
  rejection?: string | null;
  result?: ImportFilesResult;
  error?: unknown;
}) {
  // 外部能力的调用顺序。
  const calls: string[] = [];
  // 用户可见的通知记录。
  const notices: { message: string; variant: string }[] = [];
  // 实际提交的导入参数。
  let submitted: { paths: string[]; relPath: string } | null = null;
  // 当前测试使用的导入端口。
  const dragPort: DragUploadPort = {
    rejectionReason: vi.fn(() => {
      calls.push("guard");
      return options.rejection ?? null;
    }),
    targetRelPath: vi.fn(() => "docs/inner"),
    importFiles: vi.fn(async (paths, relPath) => {
      calls.push("import");
      submitted = { paths, relPath };
      if (options.error) throw options.error;
      return options.result ?? importResult();
    }),
    refresh: vi.fn(async () => {
      calls.push("refresh");
    }),
    notify: vi.fn((message, variant) => {
      notices.push({ message, variant });
    }),
  };

  return {
    calls,
    notices,
    getSubmitted: () => submitted,
    dragPort,
  };
}

describe("拖拽导入编排", () => {
  it("空路径数组不产生任何动作", async () => {
    const fixture = port({});

    await expect(runDragImport([], fixture.dragPort)).resolves.toBeNull();
    expect(fixture.calls).toEqual([]);
    expect(fixture.notices).toEqual([]);
  });

  it("守卫拒绝时只发警告通知，不触碰后端", async () => {
    const fixture = port({ rejection: "正在读取云端文件，请稍后再试" });

    await expect(
      runDragImport(["/tmp/a.txt"], fixture.dragPort),
    ).resolves.toBeNull();
    expect(fixture.calls).toEqual(["guard"]);
    expect(fixture.notices).toEqual([
      { message: "正在读取云端文件，请稍后再试", variant: "warning" },
    ]);
  });

  it("全部导入成功时按当前文件夹相对路径提交并刷新列表", async () => {
    const fixture = port({ result: importResult({ imported: 3 }) });

    const outcome = await runDragImport(["/tmp/a.txt", "/tmp/b"], fixture.dragPort);

    expect(outcome).toEqual({ imported: 3, failed: 0 });
    expect(fixture.getSubmitted()).toEqual({
      paths: ["/tmp/a.txt", "/tmp/b"],
      relPath: "docs/inner",
    });
    expect(fixture.calls).toEqual(["guard", "import", "refresh"]);
    expect(fixture.notices).toEqual([
      { message: "已导入 3 项，正在后台同步到云端", variant: "success" },
    ]);
  });

  it("部分失败时通知成功与失败数量及首个失败原因", async () => {
    const fixture = port({
      result: importResult({
        imported: 2,
        failures: [
          { source: "/tmp/a.txt", reason: "目标已存在同名项，拒绝覆盖：a.txt" },
          { source: "/tmp/b.txt", reason: "目标已存在同名项，拒绝覆盖：b.txt" },
        ],
      }),
    });

    const outcome = await runDragImport(["/tmp/a.txt", "/tmp/b.txt", "/tmp/c.txt"], fixture.dragPort);

    expect(outcome).toEqual({ imported: 2, failed: 2 });
    expect(fixture.notices).toEqual([
      {
        message: "已导入 2 项，2 项未导入：目标已存在同名项，拒绝覆盖：a.txt",
        variant: "warning",
      },
    ]);
  });

  it("全部失败时以错误语义通知首个失败原因", async () => {
    const fixture = port({
      result: importResult({
        failures: [{ source: "/tmp/a.txt", reason: "导入源命中跳过规则：a.tmp" }],
      }),
    });

    const outcome = await runDragImport(["/tmp/a.txt"], fixture.dragPort);

    expect(outcome).toEqual({ imported: 0, failed: 1 });
    expect(fixture.notices).toEqual([
      { message: "导入失败：导入源命中跳过规则：a.tmp", variant: "error" },
    ]);
  });

  it("后端整体异常时归一为错误通知且不刷新列表", async () => {
    const fixture = port({ error: new Error("读取导入源失败：磁盘不可用") });

    await expect(
      runDragImport(["/tmp/a.txt"], fixture.dragPort),
    ).resolves.toBeNull();
    expect(fixture.calls).toEqual(["guard", "import"]);
    expect(fixture.notices).toEqual([
      { message: "导入失败：读取导入源失败：磁盘不可用", variant: "error" },
    ]);
  });
});
