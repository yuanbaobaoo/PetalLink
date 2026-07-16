# 编码规则

> 所有回复使用**中文**，代码注释使用**简短、准确的中文**，代码标识符和专有技术名词保留标准英文写法。

> **前端开发必须遵循 `design-rules.md` 中定义的设计规则**

---

## 一、通用规则

### 1.1 语言与术语

- 变量、函数、类型、字段、文件名和模块名使用英文；引用现有标识符时必须保持源码原样，并使用反引号包裹。
- 专有技术名词不得生硬翻译。
- 协议字段和服务端标识保持官方大小写，例如 `fileId`、`parentFolder`、`serverId`、`uploadId`、`session_url`、`nextCursor`、`newStartCursor`。
- 禁止为了“中文化”而创造含义模糊的译名，例如把 `BFS` 写成“广度优先”、把 `PATCH` 写成“更新写入”、把 `token` 写成“令牌”。

### 1.2 可审阅性

- 代码首先服务于人工审阅：职责边界清楚，命名直接，控制流可顺序阅读，重要约束在靠近实现的位置说明。
- 生产 Rust 文件原则上不超过 **1000 行**；达到上限前应按单一职责拆分。禁止继续堆叠成超过 1000 行的综合文件。
- 门面文件只保留模块声明、公开导出、共享类型/常量和少量编排；具体读取、写入、协议解析、持久化、恢复等职责放入子模块。
- 拆分文件不得顺便改变业务逻辑。函数体、SQL、HTTP 请求、锁和 `await` 顺序、错误映射、日志语义及可见性必须保持不变。
- 避免无意义抽象。只有当拆分能形成稳定职责边界、降低文件长度或让控制流更易审阅时才新增模块。

---

## 二、Vue/TypeScript 编码规范

### 2.1 `<script setup>` 代码组织顺序（强制）

所有 `.vue` 文件统一使用 `<script setup lang="ts">`，代码按以下顺序组织：

```
① import 语句
② const / let / ref / reactive / computed 声明
③ defineProps / defineEmits
④ 生命周期钩子（onMounted、onBeforeUnmount 等）
⑤ defineExpose（如有）
⑥ watch 监听器
⑦ function 声明
```

**注意**
在声明const时，如果是单行声明的，则不用空行；多行声明则需要换行。

```
// 单行声明的，不需要空行
const containerRef = ref<HTMLElement | null>(null);
// 单行声明的，不需要空行
const isFullscreen = ref(false);

// 多行声明的，必须空行
const props = defineProps<{
	cameraData?: CameraData | null;
}>();

// 多行声明的，必须空行
const emit = defineEmits<{
	(e: "click-camera", camera: CameraData): void;
}>();

...
```

### 2.2 注释风格

| 目标 | 格式 | 要求 |
|------|------|------|
| const / let 变量 | `//` 单行注释 | **所有** const/let 都必须加，包括 useRoute()、ref()、computed() |
| function 声明 | `/** */` JSDoc | `/**` 后必须换行 |
| watch / onMounted | `/** */` 多行 | `/**` 后必须换行 |
| enum 枚举成员 | `//` 或 `/** */` 多行 | 禁止 `/** 中文 */` 单行形式 |

**禁止：**
- `/** 摄像机列表 */` 用于 const → 必须用 `//`
- `/** 获取列表 */` 单行 JSDoc → `/**` 后必须换行

### 2.3 函数声明

- 最外层独立函数必须用 `function` 声明，禁止 `const fn = () => {}`
- 函数内部回调一律用箭头函数（`computed`、`Array.map/filter/forEach`、`.catch`、`setTimeout` 等）
- Promise 一律 `async/await` + `try/catch`，禁止 `.then/.catch` 链式调用

### 2.4 watch 写法

```typescript
// ✓ 正确：紧凑写法
watch(() => props.modelValue, (newVal) => {
    // handle change
});

// ✗ 禁止：拆成多行参数
watch(
    () => props.modelValue,
    (newVal) => { /* ... */ }
);
```

### 2.5 常量规范

- 模块级常量使用 `UPPER_SNAKE_CASE`：`FILTER_OPS`、`NUMBER_TYPES`
- 响应式状态使用 `camelCase`：`loading`、`tableList`、`showConfigModal`
- 私有模块变量使用 `_camelCase` 前缀：`_sqlBlurTimer`、`_pivotKeyCounter`
- 魔法字符串必须抽为常量，禁止硬编码
- 魔法字符串抽为 `const` 常量（大写+下划线）
- 公用常量放 `common/types/`

### 2.6 模板内样式规范

- **禁止**在模板内直接写带单位的 style 属性（如 `style="width: 12px; height: 12px"`）
- **允许**仅在模板内写颜色等简单样式（如 `style="color: #666; background-color: #f5f7fa"`）
- **推荐**带单位的样式统一在 `<style scoped>` 内定义

### 2.7 defineProps / defineEmits 泛型风格

```typescript
// ✓ 推荐：TypeScript 泛型 + withDefaults
const props = withDefaults(defineProps<{
    title: string;
    required?: boolean;
}>(), {
    required: false,
});

// ✓ 推荐：Emits 泛型注解
const emit = defineEmits<{
    (e: "close"): void;
    (e: "submit", data: FormData): void;
}>();
```

### 2.8 reactive 集中状态管理

对于有 3 个以上响应式状态的组件，推荐使用 `reactive` 集中管理：

```typescript
const state = reactive({
    show: false,
    loading: false,
    editMode: false,
    data: null as UserVO | null,
});
```

- 简单状态（1-2 个）继续用 `ref` 即可
- 集中式 `state` 对象便于一眼看清组件所有状态

### 2.9 函数命名约定

| 前缀 | 用途 | 示例 |
|------|------|------|
| `fetch` | 获取数据（网络请求） | `fetchTableList()` |
| `load` | 加载数据（可能含缓存） | `loadSubmissions()` |
| `handle` | 事件处理 | `handleSubmit()`、`handleFileChange()` |
| `on` | 组件事件回调 | `onMenuSelect()` |
| `do` | 执行操作 | `doLogout()`、`doDelete()` |
| `open` | 打开弹窗/页面 | `openEditRow()` |
| `close` | 关闭弹窗 | `closeModal()` |

### 2.10 JSDoc 函数注释

对外暴露的函数（`defineExpose` 的方法、composable 导出的函数）必须加 JSDoc：

```typescript
/**
 * 打开编辑弹窗
 *
 * @param row - 要编辑的行数据
 */
function openEditRow(row: Record<string, unknown>) { ... }
```

内部使用的私有函数可不加 JSDoc，但复杂逻辑建议用 `//` 行注释说明。

### 2.11 Vue 模板中 el-tag 格式规范

**禁止：** 紧凑的一行式或属性不对齐的格式

```vue
<!-- ✗ 禁止：属性不对齐且内容在同一行 -->
<el-tag
     v-for="t in item.detectionTargets"
     :key="t"
     size="small"
     type="info"
>{{ detectionTargetMap[t] || t }}</el-tag>
```

**推荐格式 1（长度不超过一页）：** 属性和内容对齐

```vue
<!-- ✓ 正确：属性对齐，内容对齐 -->
<el-tag v-for="t in item.type" :key="t" size="small" >
    {{ algorithmTypeMap[t] || t }}
</el-tag>
```

**推荐格式 2（长度超过一页需要换行）：** 增加缩进的对齐格式

```vue
<!-- ✓ 正确：增加缩进，属性对齐，内容对齐 -->
<el-tag
    v-for="t in item.detectionTargets"
    :key="t"
    size="small"
    type="info"
>
    {{ detectionTargetMap[t] || t }}
</el-tag>
```

---

## 三、Rust 编码规范

### 3.1 声明注释（强制）

- Rust 源码中的命名声明必须有注释，包括 `fn`、实现方法、`trait`、`struct`、`enum`、`type`、`union`、`macro`、`mod`、关联常量和 `static`。
- 文件或模块职责使用 `//!`；命名声明使用 `///`；函数内部的重要分支、状态转换、不变量和安全边界使用 `//`。
- `struct` 字段和 `enum` variant 在属于公开合同、状态机或含义不直观时必须逐项说明；显而易见的私有数据容器字段不机械补注释。
- 属性应放在文档注释之后、声明之前，保持 `///` 与目标声明紧邻。
- 测试函数同样需要说明验证的核心合同，不能只把函数名翻译一遍。

### 3.2 注释内容

- 注释重点回答“为什么存在、保证什么、失败时怎样、有什么副作用”，不要复述函数名或逐行翻译代码。
- 注释短小精干。能用一句话说明时不要写成一段；复杂协议可用模块注释集中说明，避免在每个方法重复背景。
- 对 HTTP 写操作、状态机转换、锁边界、重试、恢复、数据提交等高风险逻辑，必须说明成功判定和禁止事项。
- 不为局部变量、简单闭包、显而易见的赋值机械添加注释；这类注释会增加噪声，降低审阅效率。
- 注释必须与代码同步。移动或拆分实现时，同时更新路径、职责和术语，禁止保留失效说明。

### 3.3 模块拆分

- 优先按领域职责拆分，而不是按行数平均切块。例如：`read` / `write`、`request` / `response`、`admission` / `execution` / `settlement`。
- 父模块保留稳定公开 API；子模块使用满足实现所需的最小可见性，避免因拆分扩大公共接口。
- 同一类型的多个 `impl` 可以分布在职责明确的子模块中，但调用方路径和行为必须保持稳定。
- 拆分前后应使用 diff 核对，确保变化仅限文件归属、模块声明、导入和注释；不得夹带业务修复或行为优化。

---

## 四、测试规范

- 只保留核心业务合同、协议边界、状态机、恢复语义和高风险回归测试；删除重复、低价值或只验证实现细节的测试。
- 能通过公开接口验证的 Rust 测试统一放在根目录 `tests/`，按领域使用 `*_test.rs` 命名。
- 只有确实依赖私有实现且无法通过公开合同覆盖的核心测试，才允许保留在 `src/` 内；不能移动且不属于核心测试的用例直接删除。
- 真实云端、真实账号或人工环境测试也放在 `tests/`，必须使用 `#[ignore]`，并通过明确的环境变量显式启用。默认测试不得访问真实外部服务或产生外部副作用。
- 不维护容易失真的固定测试数量；文档统一以 `cargo test -- --list` 或实际命令输出为准。
- 常规验证至少包括：`cargo fmt --all -- --check`、`cargo check --all-targets`、`cargo test --all-targets --no-run`。涉及行为时再运行相关测试和 `cargo clippy --all-targets -- -D warnings`。

### 4.1 前端测试（Vitest）

- **所有前端测试统一放在 `app/tests/` 目录下，禁止散落在 `app/stores/`、`app/api/`、`app/views/` 等业务模块目录中。** 项目根的 `tests/` 是 Rust 集成测试目录，前端测试不得放入。
- 测试文件使用 `*.test.ts` 命名；跨语言合同测试使用 `*.contract.test.ts`。
- 测试内部引用被测模块统一走 `@` 别名（如 `@/stores/sync`、`@/api/transfer`），不写相对路径，避免目录迁移后失效。需要读取 Rust 源码做合同校验时，相对路径写 `../../src/...`（`app/tests/` 上两级即项目根 `src/`）。
- 运行命令：`npm test`（在 `app/` 目录下执行 `vitest run`）。

---

## 五、文档同步规范

- 源码是当前实现的唯一事实来源。模块路径、命令、测试入口、配置默认值或目录结构变化时，必须在同一批改动中更新 `README.md` 和 `docs/` 中的现行文档。
- API 文档应指向实际实现文件；门面文件只负责声明子模块时，不得继续把方法标成位于门面文件。
- 对容易变化的测试数量、组件数量和内部文件数量，优先记录查询命令或事实来源，不硬编码统计值。
- 历史决策可以保留当时背景，但不得用现在时冒充当前结构或当前运行方式。
- 文档中的专有技术名词遵循本文件“语言与术语”规则。

---

## 六、Git 提交规范

**禁止自动提交代码。** 所有代码改动完成后，由用户明确指示（如"提交代码"、"commit"）后才能执行 `git commit`，没有指示则保持工作区状态。

- 不主动切换分支，除非用户明确要求。
- 改动完成后，告知用户改动文件清单和验证情况，由用户决定是否提交。
- 不主动执行 `git add` / `git commit` / `git push`，除非用户明确要求。
- 用户只说"提交"时仅提交，不附带 push；需要 push 时由用户另行指示。
- 用户要求 commit 时，commit message 必须以中文为主要语言编写。
- **`docs/superpowers/` 目录永不纳入版本控制**（已在 `.gitignore`）。禁止用 `git add -f` 强制添加该目录下的任何文件；该目录仅作本地工作文件，不入库。
