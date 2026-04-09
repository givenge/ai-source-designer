# AI Source Designer

AI Source Designer 是一个独立于 Reader-Rust 的 AI 书源设计工具，定位为桌面应用。项目计划基于 Tauri 构建，内置 Playwright 运行能力，并接入大模型。用户先在应用内配置 AI 服务 Base URL、API Key 和模型，再在任务页填写目标站点 URL，工具随后分析站点结构，生成兼容 Reader-Rust 的书源 JSON，并输出调试证据与验证结果。

## 当前状态

当前目录已进入可运行的桌面应用阶段：

- `src-tauri/` 已初始化并接入 Tauri 命令层
- 根目录已建立 `pnpm` + Vite + Vue 3 工程配置
- 设置页、任务页、任务详情页和基础状态管理已落地
- 首页已改成轻量入口，没有左侧长工具栏
- 任务详情页已改成子页面，左上角提供返回按钮
- 任务支持后台执行、轮询刷新和失败后继续执行
- Rust 命令层、SQLite、Playwright Worker、本地验证器和导出链路已联通
- 当前已完成 fixture 驱动的首个本地闭环，并补了真实站点 smoke 能力

当前重点不是做大规模 UI 美化，而是继续补强采样、验证和修复的稳定性。

## 项目目标

- 做成一个本地运行的桌面应用
- 用户先配置 AI 服务 Base URL、API Key 和模型，再输入目标站点 URL 启动任务
- 内置 Playwright，不要求用户自行安装浏览器自动化环境
- 自动生成兼容 Reader-Rust 的书源 JSON
- 调用本地验证器确认规则可执行，而不是只输出“看起来正确”的结果
- 输出调试报告、截图、日志和失败原因，支持人工修正

## 非目标

- 不保证任意网站一次生成成功
- 不把强反爬、验证码、复杂签名站点作为第一阶段目标
- 不在 v1 默认引入向量数据库
- 不把 JavaScript 规则作为默认首选
- 不先做 Web SaaS

## 与 Reader-Rust 的关系

本项目和 Reader-Rust 是协作关系，不是子模块关系。

- Reader-Rust 负责书源执行、搜索、目录和正文解析
- AI Source Designer 负责桌面端交互、站点分析、规则生成、验证和导出
- 输出格式必须兼容 Reader-Rust 当前的 `BookSource` 与 `Rule` 模型

主要参考文件：

- [DEVELOPMENT.md](./DEVELOPMENT.md)
- [AGENTS.md](./AGENTS.md)
- [TASKS.md](./TASKS.md)
- [Reader-Rust BookSource](../reader-rust/src/model/book_source.rs)
- [Reader-Rust Rule](../reader-rust/src/model/rule.rs)

## 推荐实施方向

第一版建议采用 Tauri + 内置 Playwright 的桌面方案：

1. Tauri 前端负责 AI 配置管理、目标站点 URL 输入和任务可视化
2. Tauri Core 负责本地任务调度、存储和日志
3. 内置 Playwright Worker 负责页面采样和网络抓包
4. 模型适配层负责生成规则候选
5. 本地验证器负责判断规则是否真的可用

## 建议的默认技术栈

如果新线程直接开始搭项目，建议默认采用以下技术组合：

- Tauri 2
- Rust
- Vue 3 + TypeScript
- 内置 Playwright Worker
- SQLite
- `~/.aisourcedesign/settings.json` 统一配置存储

如果没有明确的新要求，默认优先选择这套栈，不要先引入服务端拆分或云端复杂度。

## 当前目录结构

当前仓库已采用如下布局：

```text
src/
  assets/
  components/
  lib/
  pages/
  stores/
  test/
  types/
src-tauri/
  src/
playwright/
  worker/
fixtures/
  novel-world/
artifacts/
```

说明：

- `src/`：桌面前端页面、组件、状态管理和类型定义
- `src-tauri/`：Rust 核心、命令层、任务调度和验证器
- `playwright/worker/`：浏览器侧车协议和采样实现
- `fixtures/`：本地 fixture 站点、mock 响应和样本产物
- `artifacts/`：运行期任务证据与导出结果

## 开发命令

统一使用以下命令：

- `pnpm dev`
- `pnpm build`
- `pnpm test`
- `pnpm lint`
- `pnpm tauri dev`
- `pnpm tauri build`

首次拉起前需要先安装依赖。

当前已验证通过的命令：

- `pnpm build`
- `pnpm test`
- `pnpm tauri build --debug`
- `pnpm smoke:live -- 'https://www.gutenberg.org/ebooks/search/?query=sherlock' --label gutenberg-search`

## 本地数据位置

桌面应用运行数据统一写入 `~/.aisourcedesign`：

- `settings.json`：AI 配置（`baseUrl`、`model`、`apiKey`）
- `app.sqlite`：任务索引
- `logs/`：应用日志
- `tasks/<task_id>/`：截图、DOM、网络样本、任务日志与导出 JSON

## Live Smoke

为了把 fixture 闭环往真实站点推进，仓库现在提供开发态 smoke 命令：

- `pnpm playwright:install`
- `pnpm smoke:live -- <url>`

示例：

```bash
pnpm playwright:install
pnpm smoke:live -- https://www.gutenberg.org/ebooks/search/?query=sherlock
```

说明：

- 该命令会直接调用 `playwright/worker/index.mjs`
- 采样产物会写入 `artifacts/live-smoke/...`
- 当前已实际验证过的样例产物目录：`artifacts/live-smoke/gutenberg-search/`
- 如果配置了以下环境变量，还会顺带做一次真实 OpenAI-compatible 模型冒烟：
  - `AI_SOURCE_DESIGNER_BASE_URL`
  - `AI_SOURCE_DESIGNER_MODEL`
  - `AI_SOURCE_DESIGNER_API_KEY`
- 当前 smoke 主要用于开发态验证真实站点采样和模型接入，不替代桌面应用中的正式验证闭环

## 已落地的实现合同

目前代码与文档已经对齐以下合同：

- 任务状态机：`draft -> sampling -> synthesizing -> validating -> repairing -> completed|failed`
- 任务详情页支持失败后继续执行
- 产物目录：`tasks/<task_id>/screenshots|dom|network|logs|output`
- 导出字段只覆盖 `searchUrl`、`ruleSearch`、`ruleBookInfo`、`ruleToc`、`ruleContent`
- 兼容性以 `../reader-rust/src/model/book_source.rs` 和 `../reader-rust/src/model/rule.rs` 为准
- `ruleContent.title`、`ruleToc.updateTime` 不进入本项目 v1 输出

## 下一步

- 先阅读 [TASKS.md](./TASKS.md)，按 Phase 0 启动项目
- 初始化 Tauri 桌面应用骨架
- 落地 AI Base URL / API Key / 模型配置，以及目标站点 URL 输入流程
- 接入内置 Playwright
- 实现与 Reader-Rust 的字段兼容层和本地验证闭环
