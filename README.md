# AI Source Designer

AI Source Designer 是一个独立于 [Reader-Rust](https://github.com/givenge/reader-rust) 的 AI 书源设计工具，定位为桌面应用。用户先在应用内配置 AI 服务 Base URL、API Key 和模型，再在任务页填写目标站点 URL，工具会分析站点结构，生成兼容 [Reader-Rust](https://github.com/givenge/reader-rust) 的书源 JSON，并输出调试证据与验证结果。

## 当前状态

当前目录已进入可运行的桌面应用阶段：

- `src-tauri/` 已初始化并接入 Tauri 命令层
- 根目录已建立 `pnpm` + Vite + Vue 3 工程配置
- 设置页、任务页、任务详情页和基础状态管理已落地
- 首页已改成轻量入口，没有左侧长工具栏
- 任务详情页已改成子页面，左上角提供返回按钮
- 首页默认入口为 Agent 全流程，Classic 仅放在高级/调试模式里
- 任务支持后台执行、轮询刷新和失败后继续执行
- Rust 命令层、SQLite、Playwright Worker、本地验证器和导出链路已联通
- 当前已完成 fixture 驱动的首个本地闭环，并补了真实站点 smoke 能力
- 任务详情页已补充任务概览、验证摘要、规则快照和任务/worker 日志切换

## 与 [Reader-Rust](https://github.com/givenge/reader-rust) 的关系

本项目和 [Reader-Rust](https://github.com/givenge/reader-rust) 是协作关系，不是子模块关系。

- [Reader-Rust](https://github.com/givenge/reader-rust) 负责书源执行、搜索、目录和正文解析
- AI Source Designer 负责桌面端交互、站点分析、规则生成、验证和导出
- 输出格式必须兼容 [Reader-Rust](https://github.com/givenge/reader-rust) 当前的 `BookSource` 与 `Rule` 模型

主要参考文件：

- [DEVELOPMENT.md](./DEVELOPMENT.md)
- [AGENTS.md](./AGENTS.md)
- [TASKS.md](./TASKS.md)
- [Reader-Rust 仓库](https://github.com/givenge/reader-rust)
- [Reader-Rust BookSource](../reader-rust/src/model/book_source.rs)
- [Reader-Rust Rule](../reader-rust/src/model/rule.rs)

## 当前目录结构

当前仓库已采用如下布局：

```text
src/
  前端页面 / 组件 / 状态管理 / 类型定义
src-tauri/
  Rust 命令层 / 任务调度 / 验证器 / Worker 协议
playwright/
  worker/
    Playwright 侧车采样实现
fixtures/
  本地样本站点 / mock 响应 / fixture 证据
scripts/
  live-smoke.mjs
    开发态真实站点采样脚本
artifacts/
  运行期证据与导出结果
dist/
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
- 任务详情页提供任务概览、验证摘要、规则快照、任务日志和 worker 日志切换
- Agent 对话修复默认回传模型响应内容，并在失败后允许继续修复
