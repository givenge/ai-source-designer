# AGENTS.md

本文件为 AI Source Designer 项目中的代理协作说明。新线程进入本目录后，应优先阅读本文件，再查看 `DEVELOPMENT.md` 与 `TASKS.md`。

## 项目概述

AI Source Designer 是一个独立项目，用于根据网站自动生成兼容 Reader-Rust 的书源 JSON。项目目标不是做成网页服务，而是做成一个本地运行的 Tauri 桌面应用。核心流程不是“让模型直接写一份 JSON”，而是建立以下闭环：

1. 用户先配置 AI 服务 Base URL、API Key 和模型，再输入目标站点 URL
2. 内置 Playwright 采样页面与网络请求
3. 生成规则候选
4. 调用本地验证器真实测试
5. 根据失败原因修复规则

## 与 Reader-Rust 的边界

- 本项目独立于 `reader-rust`
- 不要默认修改 `../reader-rust` 中的代码，除非用户明确要求
- 输出格式必须兼容 `../reader-rust/src/model/book_source.rs`
- 规则字段必须兼容 `../reader-rust/src/model/rule.rs`
- 规则语法与行为必须参考 `../reader-rust/docs/book-source/`
- 如需做最终兼容验证，可选接入 Reader-Rust 的 `/reader3/testBookSource`

## 当前阶段

当前仓库已进入可运行桌面应用阶段。新线程仍应先检查目录和文档，再决定是否需要继续初始化或修改实现。

## 默认技术假设

如果用户没有指定技术栈，默认采用以下方案：

- Tauri 2
- Rust 负责本地核心和命令层
- Vue 3 + TypeScript 负责桌面前端
- Playwright Worker 作为应用内置运行能力
- SQLite 作为本地任务和结果存储

不要默认把项目实现成 Web SaaS，也不要默认拆成多个远程服务。

## 核心约束

- v1 做桌面应用，不做网页后台
- v1 必须内置 Playwright，不要求用户自行安装 Node.js 或 Playwright
- v1 用户主流程分成两类输入：AI 配置（Base URL、API Key、模型）和任务输入（目标站点 URL）
- API Key 与基础配置一起写入 `~/.aisourcedesign/settings.json`，但不得落到普通日志中
- v1 不引入向量数据库
- v1 不以 JavaScript 规则为首选方案
- 优先顺序为：JSON API > CSS > XPath > JavaScript
- 生成结果必须可验证，不接受“模型判断应该可用”
- 所有重要步骤应保留证据：截图、DOM 片段、请求样本、验证日志
- 对失败站点要输出清晰失败原因，不要静默失败

## 通用性硬约束（重点）

- 首要目标是“通用性”，不是“单站点成功率”
- 禁止在核心流程中做站点特调：不得基于特定域名、站点名、固定路径写 if/else 逻辑
- 允许站点差异适配，但必须通过通用机制实现：证据驱动、规则推断、验证失败后修复，而不是硬编码
- `worker` 定位为纯执行层：只执行明确动作，不内置站点策略判断
- `agent` 负责决策与参数编排：搜索、详情、目录、正文、书海探索都通过统一协议下发给执行层
- 当搜索链路不稳定时，不应直接判源不可用；`search` 与 `explore` 至少一条可用即可进入后续详情/目录/正文验证
- 默认入口应使用 `agent`，`classic` 只作为高级/调试模式保留，不要把它当成常规主流程
- 任何修复应优先提升跨站复用能力；如果一个修复只能覆盖单站，应视为临时补丁并标注后续通用化任务
- 新增或修改规则字段时，优先对齐 Reader-Rust 当前实现，不跟随旧示例或站点私有写法
- 验收标准应包含“换站不改代码可运行”的检查，至少在多个结构不同的站点上手工冒烟

## 建议目录

如果需要初始化项目，优先保持结构简洁。可以从以下布局开始：

```text
src/
  pages/
  components/
  stores/
src-tauri/
  src/
playwright/
  worker/
fixtures/
artifacts/
```

说明：

- `src/` 负责桌面前端页面和组件
- `src-tauri/` 负责 Tauri Rust 核心
- `playwright/worker/` 负责 Playwright 侧车进程
- `fixtures/` 存站点样本
- `artifacts/` 存截图、日志和任务证据

## 开发优先级

实现顺序应优先保证下面五件事：

1. 能启动 Tauri 桌面应用
2. 能让用户配置 AI Base URL、API Key、模型，并输入目标站点 URL
3. 能调用内置 Playwright 打开网页并采样
4. 能输出最小可用书源 JSON
5. 能通过本地验证器判断规则是否真的可用

在这几步稳定前，不要过早做以下工作：

- 大规模 UI 美化
- 多用户系统
- 云端同步
- 独立向量数据库
- 大量历史样本召回系统

## 命令约定

如果你要初始化或维护脚手架，请在项目根目录提供统一命令，并尽量遵守以下命名：

- `pnpm dev`
- `pnpm build`
- `pnpm test`
- `pnpm lint`
- `pnpm tauri dev`
- `pnpm tauri build`

如果命令尚不存在，应优先补齐，再继续实现功能。

## 输出要求

任何实现都应优先围绕以下交付物展开：

- 可导出的书源 JSON
- 每步验证结果
- 可读的失败原因
- 可复用的任务证据
- 可查看的截图和日志
- 任务详情页需要同时展示任务概览、验证摘要、规则快照和日志切换
- Agent 对话修复应显示模型响应内容，而不是只显示调用日志

## 参考文件

- `./README.md`
- `./DEVELOPMENT.md`
- `./TASKS.md`
- `../reader-rust/src/model/book_source.rs`
- `../reader-rust/src/model/rule.rs`
- `../reader-rust/docs/book-source/`
- `../reader-rust/docs/api/book-source.md`
