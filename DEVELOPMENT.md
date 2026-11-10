# AI 书源设计工具开发文档

本文档用于规划一个独立于 Reader-Rust 主项目之外的 AI 书源设计工具。该工具定位为桌面应用，基于 Tauri 构建，内置 Playwright 运行能力，并接入大模型。用户在应用内先配置 AI 服务 Base URL、API Key 和模型，再在任务页填写目标站点 URL，工具随后生成可被 Reader-Rust 导入的书源 JSON，并对结果做真实测试与修复。

当前状态：项目已进入可运行桌面应用阶段，本文件作为架构与实施路线的源文档使用。

2026-11-10 运行时决策更新：用户要求改回系统 Playwright。当前实现随附 Worker 脚本，使用系统 Node.js、已修复的用户目录 Playwright 或本机依赖，并在设置页提供检测、一键修复及安装日志。修复先安装到暂存目录，通过 Chromium 启动验证后启用，失败保留原运行环境。本文件后续“内置 Node.js/Playwright/Chromium”分发方案属于早期规划，当前以此决策及 AGENTS.md 为准。

## 项目定位

### 目标

- 做成一个本地运行的桌面应用，而不是先做 Web SaaS
- 用户先配置 AI 服务 Base URL、API Key 和模型，再输入目标站点 URL 启动任务
- 应用内置 Playwright 运行能力，不要求用户额外安装浏览器自动化环境
- 自动识别搜索、详情、目录、正文链路
- 生成兼容 Reader-Rust 的书源 JSON
- 对生成结果做自动验证，输出可导入、可调试、可继续修正的结果
- 降低书源开发门槛，提高书源调试效率

### 非目标

- 不承诺对任意站点一次生成成功
- 不处理必须登录、验证码、复杂签名、强反爬站点的全自动接入
- 不把大模型输出直接视为最终答案，必须经过工具验证
- 不在第一阶段引入向量数据库
- 不在第一阶段做云端多用户协作平台

### 产品定义

这个工具更准确的定位是“本地桌面版 AI 辅助书源生成与验证器”，不是单纯的“聊天式生成器”。真正决定成功率的关键不是模型会不会写 JSON，而是是否具备以下闭环：

1. 发现站点结构
2. 采样页面与网络请求
3. 生成规则候选
4. 真实执行验证
5. 根据失败原因自动修复

## 用户交互设计

### 首版核心输入

第一版应把输入拆成两类：AI 配置和任务输入。

| 字段 | 必填 | 说明 |
|------|------|------|
| AI 服务 Base URL | 是 | 用于调用大模型的接口地址 |
| API Key | 是 | 用于调用大模型 |
| 模型 | 是 | 由用户手动选择，例如 GPT-4.1 / GPT-5 系列等 |
| 目标站点 URL | 是 | 要分析并生成书源的网站地址 |

### 建议的首版界面

首版桌面应用建议至少包含以下区域：

- 任务创建页
- 任务运行状态页
- 证据与日志面板
- 生成结果与导出面板
- 设置页

### 任务创建流程

1. 用户输入目标站点 URL
2. 用户确认当前 AI 配置可用
3. 默认入口使用 Agent 全流程，Classic 仅作为高级/调试模式保留
4. 用户点击“开始分析”
5. 应用启动内置 Playwright，采样页面和网络请求
6. 应用调用模型生成规则候选
7. 应用执行本地验证与修复
8. 用户查看 JSON、日志、任务概览、规则快照和截图，并导出结果

### 高级设置

建议把 AI 配置放到单独的设置页，任务页只放目标站点 URL。除核心字段外，第一版可以预留以下高级项：

- 测试关键词
- 代理设置
- 超时时间
- Reader-Rust 兼容性校验开关

任务模式建议默认只展示 Agent 全流程；Classic 固定采样应收进高级/调试模式，用于回归测试、采样排障和协议调试，不作为常规入口。

## 为什么做成独立项目

将 AI 书源工具独立于 Reader-Rust 本体有几个好处：

- 主项目继续保持“阅读服务器”职责，避免引入桌面应用和浏览器自动化复杂度
- AI 工具可以独立升级 Tauri、模型适配层和 Playwright 运行时
- 书源生成失败不会影响现有阅读链路
- 后续可以支持更多目标格式，而不局限于当前项目

推荐形态：

- `reader-rust`: 继续负责书源执行、书籍搜索、目录与正文解析
- `ai-source-designer`: 负责桌面端交互、站点分析、规则生成、验证与导出

## 输出目标

工具的最终输出必须与当前 Reader-Rust 数据结构兼容，核心参考资料如下：

- [Reader-Rust 书源开发概述](../reader-rust/docs/book-source/index.md)
- [BookSource 模型](../reader-rust/src/model/book_source.rs)
- [Rule 模型](../reader-rust/src/model/rule.rs)

### 目标对象

最小可用输出应覆盖以下字段：

```json
{
  "bookSourceName": "示例书源",
  "bookSourceUrl": "https://example.com",
  "enabled": true,
  "searchUrl": "https://example.com/search?q=${key}&page=${page}",
  "ruleSearch": {
    "bookList": ".book-item",
    "name": "@css::is(h3 a)@text",
    "author": ".author@text",
    "bookUrl": "@css::is(h3 a)@href"
  },
  "ruleBookInfo": {
    "name": "h1@text",
    "author": ".book-author@text",
    "intro": ".intro@text",
    "tocUrl": ".catalog@attr[href]"
  },
  "ruleToc": {
    "chapterList": ".chapter-list li",
    "chapterName": "a@text",
    "chapterUrl": "a@attr[href]"
  },
  "ruleContent": {
    "content": ".content@textNodes",
    "nextContentUrl": ".next-page@attr[href]",
    "replaceRegex": "广告词##"
  }
}
```

### 字段边界

以当前 Rust 模型为准，第一版需要重点支持以下字段：

| 层级 | 必要字段 | 说明 |
|------|----------|------|
| `BookSource` | `bookSourceName` `bookSourceUrl` | 书源标识 |
| `BookSource` | `searchUrl` 或 `exploreUrl` | 搜索或书海至少一条入口经过实际验证 |
| `ruleExplore` | `bookList` `name` `bookUrl` | 书海列表及分页验证 |
| `ruleSearch` | `bookList` `name` `bookUrl` | 搜索最小闭环 |
| `ruleBookInfo` | `name` `author` `intro` `tocUrl` | 书籍详情最小闭环 |
| `ruleToc` | `chapterList` `chapterName` `chapterUrl` | 目录最小闭环 |
| `ruleContent` | `content` | 正文最小闭环 |

补充说明：

- `ruleContent` 当前以后端实现为准，核心字段是 `content`、`nextContentUrl`、`replaceRegex`
- 书海从实际导航和下一页链接推断分类与分页 URL，支持路径、查询参数及首页特殊地址；采样下一页并验证新的书籍链接，固定入口或重复首页不能判为分页可用
- 未采样的书海不计入有效入口；书海验证失败但搜索可用时，修复后仍失败则关闭书海，并在日志中说明保留的能力
- 第一版不要求覆盖登录流程和复杂请求头拼装
- 能用 CSS、JSONPath、XPath、Regex 解决的问题，不应默认生成 JavaScript 规则

## 支持站点范围

第一版应只覆盖以下类型：

- 公网可访问
- 不需要登录
- 没有验证码
- 搜索链路清晰
- HTML 页面结构稳定，或能直接发现 JSON API
- 目录与正文不是强加密内容

以下类型默认判定为高风险或暂不支持：

- Cloudflare 挑战页
- 登录后才可搜索或阅读
- 请求签名经常变化
- 正文依赖混淆 JS 运行时还原
- 严格频控或人机校验

## 桌面应用架构

推荐采用 Desktop-first + Browser-first 架构：桌面应用负责任务交互和本地存储，内置浏览器自动化负责真实采样与验证。

```text
用户配置 AI Base URL / API Key / 模型
并输入目标站点 URL
            ↓
Tauri 前端界面
            ↓
Tauri 命令层（Rust）
            ↓
任务编排器
   ├─ 模型适配层
   ├─ Playwright Worker
   ├─ 规则生成器
   ├─ 本地验证器
   └─ 产物存储
            ↓
导出 JSON + 日志 + 截图 + 置信度
```

### 技术分层

#### 1. Tauri 前端

负责用户交互：

- 管理 AI 配置（Base URL、API Key、模型）
- 输入目标站点 URL
- 展示任务状态
- 展示日志、截图、规则和导出结果
- 管理设置页和历史任务列表

#### 2. Tauri Core

负责本地应用能力：

- 调度任务执行
- 调用本地 Playwright Worker
- 管理文件系统与应用数据目录
- 管理本地数据库
- 处理本地配置读写

建议由 Rust 侧统一负责任务生命周期、日志聚合和错误处理。

#### 3. Playwright Worker

负责浏览器自动化与采样：

- 打开站点首页、搜索页、详情页、目录页、章节页
- 提取 DOM 片段、链接样本、可见文本、截图
- 监听网络请求，识别 JSON API
- 保存关键页面快照，供模型引用

建议实现方式：

- 使用内置 Playwright Worker
- Worker 由桌面应用托管
- 不要求用户单独安装 Node.js 或 Playwright

### 内置 Playwright 策略

“内置 Playwright”不应理解为让用户手工配置运行环境，而应由应用自行封装。推荐策略如下：

- v1 只内置一套 Chromium 运行时，先不追求多浏览器覆盖
- Playwright Worker 作为应用内侧车进程运行
- 桌面应用在首次启动时检查浏览器依赖是否齐全
- 浏览器二进制和 Worker 版本必须跟应用版本一起管理

这样做的好处是：

- 用户安装后即可直接使用
- 运行环境更可控
- 出问题时更容易复现

代价是：

- 安装包体积会增大
- Playwright 升级会影响桌面应用打包和签名流程

#### 4. 模型适配层

负责调用大模型：

- 接收用户输入的 API Key
- 接收用户配置的 AI 服务 Base URL
- 接收用户选择的模型
- 统一构建提示词与工具调用上下文
- 输出结构化规则候选

建议第一版支持 OpenAI 兼容接口抽象，后续再扩展到更多 Provider。

#### 5. 规则生成器

负责把站点证据转成规则候选：

- 生成 `searchUrl`
- 生成 `ruleSearch`
- 生成 `ruleBookInfo`
- 生成 `ruleToc`
- 生成 `ruleContent`
- 判断是否需要 `replaceRegex` 或 `nextContentUrl`

#### 6. 验证器

负责真实验证，不接受“看起来对”：

- 校验输出 JSON 结构是否兼容 Reader-Rust
- 用采样结果和真实请求验证规则是否可执行
- 给出结构化错误原因

兼容性验证建议分成两层：

- 必选：本地兼容验证
- 可选：对接 Reader-Rust 的 `/reader3/testBookSource` 做最终验证

这样可以保证首版桌面应用只依赖本地 AI 配置和目标站点 URL，不强依赖额外的 Reader-Rust 服务地址。

#### 7. 产物存储

负责存储任务结果：

- 导出的书源 JSON
- 页面截图
- DOM 片段
- 网络请求样本
- 调试日志
- 历史任务记录

#### 8. 详情页信息密度

任务详情页不应只展示最终 JSON，还应提供：

- 任务概览：验证通过率、规则覆盖、产物数量、导出与日志计数
- 验证摘要：各阶段是否通过、失败原因、证据引用
- 规则快照：`searchUrl`、`ruleSearch`、`ruleBookInfo`、`ruleToc`、`ruleContent` 的关键字段
- 日志视图：任务日志与 worker 原始日志切换
- Agent 对话修复：展示模型响应内容，并允许失败后继续修复

## 密钥与本地存储

### API Key 存储

当前实现中，API Key 与 Base URL、模型一起写入 `~/.aisourcedesign/settings.json`。

要求：

- API Key 可由用户手动更新或清除
- UI 中默认不回显完整 Key
- 日志中不得输出明文 Key

### 本地数据目录

建议使用应用专属数据目录保存以下内容：

- 本地数据库
- 任务记录
- 截图与日志
- 导出文件

## 实现合同

为避免第一轮实现边界反复变化，首版直接锁定以下运行合同。

### 任务状态机

任务状态固定为：

```text
draft -> sampling -> synthesizing -> validating -> repairing -> completed | failed
```

说明：

- `draft`：任务已创建，尚未真正执行
- `sampling`：Playwright Worker 正在采样页面、DOM 和网络请求
- `synthesizing`：模型适配层正在根据证据生成规则候选
- `validating`：本地验证器正在按搜索、详情、目录、正文顺序验证
- `repairing`：根据失败原因进行自动修复或候选回退
- `completed`：已生成可导出结果并给出验证报告
- `failed`：任务失败，必须带结构化失败原因

### 核心数据模型

首版前后端统一使用以下对象边界：

- `AiSettings`：`baseUrl`、`model`、`apiKey`
- `TaskDraft`：`targetUrl`、`testKeyword`
- `TaskRecord`：`id`、`status`、`createdAt`、`updatedAt`、`targetUrl`、`testKeyword`、`errorSummary`
- `ArtifactManifest`：记录截图、DOM、网络样本、日志和导出 JSON 的相对路径
- `SourceCandidate`：内部候选书源、证据引用、置信度、修复历史
- `ValidationReport`：`search`、`bookInfo`、`toc`、`content`、`cleanup` 五段结果

### Tauri 命令合同

首版 Tauri 命令层只暴露四类能力：

- 设置：读取和保存 AI 配置
- 任务：创建任务、查询任务、启动任务
- 产物：读取任务日志、截图、DOM 和请求样本
- 导出：导出最终书源 JSON

### 产物目录

每个任务在应用数据目录下固定使用如下布局：

```text
tasks/<task_id>/
  screenshots/
  dom/
  network/
  logs/
  output/
```

约束：

- 所有截图都写入 `screenshots/`
- DOM 抽样片段和页面 HTML 写入 `dom/`
- 抓包样本写入 `network/`
- 运行日志写入 `logs/`
- 导出的书源 JSON 和调试报告写入 `output/`

## 兼容性说明

本项目的书源兼容性以 Reader-Rust 实际 Rust 模型为准，而不是只以文档示例为准。首版实现只覆盖最小闭环字段：

- `searchUrl`
- `ruleSearch`
- `ruleBookInfo`
- `ruleToc`
- `ruleContent`

必须注意以下兼容性陷阱：

- `ruleContent.title` 与 `ruleToc.updateTime` 已存在于 Reader-Rust 当前模型中，可按页面证据生成可选字符串规则
- `bookUrlPattern` 是详情 URL 的正则表达式，不能填入含占位符的 URL 模板；列表字段必须在单个列表项的作用域内验证
- 以标签开头的 CSS 后代选择器会统一导出为 `@css::is(...)`，避免 Reader-Rust 将其解释为旧版多 class 语法
- 任何新字段都必须先在 `../reader-rust/src/model/book_source.rs` 与 `../reader-rust/src/model/rule.rs` 中确认后再加入导出对象

第一版默认不覆盖以下能力：

- `exploreUrl`
- 登录流程
- Cookie Jar 定制
- 复杂请求头拼装
- 依赖 JavaScript 的首选规则方案

## AI 接口合同

首版模型接入固定为 OpenAI-compatible 接口抽象。

请求约束：

- 设置页保存 `baseUrl`、`model`、`apiKey`
- 配置直接写入 `~/.aisourcedesign/settings.json`
- 真实运行时调用用户配置的兼容接口
- 测试环境使用固定 mock 响应样本，避免把自动化测试绑到外网

响应约束：

- 模型输出必须为结构化候选对象
- 每个关键规则字段都要带证据引用，不能只给“建议规则”
- 模型输出不代表成功，必须进入本地验证器

## Fixture 策略

首个闭环以本地 fixture 为主，真实公开站点只做手工冒烟验证，不作为自动化稳定基线。

Fixture 至少覆盖：

- 搜索页
- 详情页
- 目录页
- 正文页
- 一个 JSON API 搜索样本
- 相对路径补全
- 正文分页
- 广告清洗

这样做的目的：

- 保证第一轮可以重复开发和回归测试
- 降低站点改版导致的误报
- 让规则生成器和验证器先围绕确定输入打磨

## 规则生成策略

### 总体原则

规则生成必须遵循“越简单越优先”的策略：

1. 优先 JSON API
2. 其次 HTML + CSS
3. 再考虑 XPath
4. 最后才使用 JavaScript

原因：

- JSON API 最稳定，字段边界最清晰
- CSS 规则最容易维护
- XPath 可读性较差，但有时对复杂结构更稳
- JavaScript 规则最灵活，也最难维护和调试

### URL 发现策略

应优先发现以下 URL：

- 搜索 URL 模板：`...${key}...${page}`
- 书籍详情页 URL
- 目录页 URL
- 章节页 URL
- 正文分页 URL

推荐做法：

- 从站内搜索表单、站内链接和网络请求中推断参数格式
- 保存多个候选模板，让验证器实际跑通后再定稿
- 对相对路径做标准化处理，统一转绝对 URL

### 选择器策略

模型输出选择器时应遵循以下约束：

- 避免依赖随机类名、动态 hash 类名
- 尽量使用稳定结构和语义类名
- 避免过深层级选择器
- 对多候选项打分，而不是只返回一个结果
- 当列表选择器不稳定时，允许结合文本特征做二次过滤

### 内容清洗策略

正文规则建议采用分层清洗：

1. 先提正文容器
2. 再保留段落与换行
3. 最后移除广告、脚本、引导文本

`replaceRegex` 只用于明确、稳定、可验证的清洗规则，不要让模型一次生成过多激进替换，避免误删正文。

## Agent 与工具设计

推荐最小能力集合如下：

| 能力 | 作用 | 是否必需 |
|------|------|----------|
| Playwright 页面打开 | 获取真实页面结构 | 是 |
| DOM 采样 | 提供规则生成证据 | 是 |
| 网络抓包 | 发现 JSON API | 是 |
| 截图 | 给人工复核和模型对齐上下文 | 是 |
| JSON Schema 校验 | 保证输出格式正确 | 是 |
| 本地兼容验证器 | 保证规则可执行 | 是 |
| Reader-Rust 最终校验 | 做高级兼容验证 | 否 |
| 代理 / 限速 | 降低被封风险 | 否 |

### 建议的 Agent 划分

#### `site-discovery`

输入站点首页，输出：

- 站点类型判断
- 搜索入口候选
- 详情页样例 URL
- 目录页样例 URL
- 正文页样例 URL

#### `network-inspector`

输入浏览器请求记录，输出：

- 是否存在可用 JSON API
- 搜索接口模板
- 详情/目录/正文接口模板
- 请求头、参数、分页模式

#### `rule-synthesizer`

输入 DOM 和网络证据，输出：

- 结构化规则候选
- 每条规则的证据说明
- 规则置信度

#### `rule-validator`

输入候选书源，输出：

- 每一步是否通过
- 搜索结果数量
- 是否能进入详情页
- 是否能拿到目录
- 是否能抓到正文
- 失败原因

#### `rule-repair`

输入失败日志，输出：

- 修正规则
- 修复理由
- 风险说明

## 验证闭环

这是整个工具最关键的部分。建议验证流程固定为五步：

1. 搜索验证
2. 详情验证
3. 目录验证
4. 正文验证
5. 分页与清洗验证

### 搜索验证

目标：

- `searchUrl` 能正确替换 `${key}` 与 `${page}`
- `ruleSearch` 至少能提取一条结果
- 每条结果必须含 `name` 和 `bookUrl`

### 详情验证

目标：

- 能提取 `name`
- 能提取 `author` 或 `intro`
- 能拿到 `tocUrl`，或者能确定目录与详情页同页

### 目录验证

目标：

- 目录至少能提取多条章节
- 每条章节应具备 `chapterName` 与 `chapterUrl`
- `chapterUrl` 需能拼出可访问的绝对路径

### 正文验证

目标：

- 能提取非空正文
- 正文长度达到最小阈值
- 不应以大量导航、脚本、广告文本为主

### 分页与清洗验证

目标：

- 若存在分页，能继续抓取下一页
- 若存在明显广告，`replaceRegex` 不应误伤正文

## 输出设计

工具最终应输出三类结果：

### 1. 可导入书源 JSON

用于直接导入 Reader-Rust。

### 2. 调试报告

建议包含以下信息：

- 输入站点
- 使用的 AI 服务 Base URL
- 使用的模型
- 识别出的搜索 URL
- 每个规则的证据来源
- 验证结果
- 失败步骤
- 修复历史
- 置信度

### 3. 中间产物

便于复盘和手工修复：

- 页面截图
- DOM 片段
- 网络请求样本
- 候选选择器列表
- 失败日志

## 数据存储建议

第一版不建议引入复杂存储系统，使用本地数据库和应用目录即可。

建议存储以下对象：

- 任务记录
- 站点快照
- 规则候选
- 验证结果
- 最终导出书源

### 是否需要向量数据库

第一版不需要。

原因：

- 早期关键问题是规则生成闭环，而不是语义召回
- 样本规模不大时，普通模板库和规则库就足够
- 引入向量库会额外增加 embedding、检索、回填和一致性复杂度

向量数据库只在以下场景值得加入：

- 已沉淀大量高质量书源样本
- 希望根据站点结构召回相似案例
- 希望自动选择 few-shot 示例
- 希望按失败模式检索历史修复策略

如果后续确实需要，优先考虑轻量方案：

- SQLite + sqlite-vec
- PostgreSQL + pgvector

## MVP 范围

第一阶段建议只实现以下能力：

- 提供桌面端任务创建页面
- 提供 AI 配置页（Base URL、API Key、模型）
- 用户输入目标站点 URL
- 内置 Playwright 打开站点并采样
- 自动发现搜索入口
- 自动生成 `searchUrl`
- 自动生成 `ruleSearch`
- 自动生成 `ruleBookInfo`
- 自动生成 `ruleToc`
- 自动生成 `ruleContent`
- 自动执行本地验证
- 输出 JSON、截图与调试报告

### 成功标准

MVP 的目标不应定义为“任意站点一键成功”，而应定义为：

- 对规则清晰的公开小说站，具备较高直出成功率
- 对结构中等复杂的站点，能生成接近可用的结果
- 对失败站点，能明确说明失败原因，而不是静默失败
- 用户安装应用后，无需自行配置 Playwright 环境即可运行

## 推荐技术选型

| 层 | 建议 |
|----|------|
| 桌面壳 | Tauri 2 |
| 桌面前端 | Vue 3 + TypeScript 或 React + TypeScript |
| 本地核心 | Rust |
| 浏览器自动化 | 内置 Playwright Worker |
| 大模型接入 | OpenAI 兼容接口抽象 |
| 输出约束 | JSON Schema |
| 本地存储 | SQLite |
| 配置存储 | `~/.aisourcedesign/settings.json` |
| 产物目录 | `~/.aisourcedesign/tasks/<task_id>/...` |

## 研发里程碑

### Milestone 1: 桌面应用骨架

- 初始化 Tauri 项目
- 建立设置页和任务页
- 接入 AI Base URL、API Key 与模型选择
- 跑通本地任务生命周期

### Milestone 2: 内置 Playwright

- 接入 Playwright Worker
- 跑通页面打开、截图、DOM 提取和网络采样
- 解决本地运行时与打包后的可执行问题

### Milestone 3: 单站点闭环

- 输入 URL 后生成单份书源
- 跑通最小验证流程
- 支持导出 JSON 和调试报告

### Milestone 4: 自动修复

- 为常见失败类型增加自动修复
- 支持多候选规则回退

### Milestone 5: 半自动调试台

- 人工可查看截图、规则和验证日志
- 人工可局部修改并重新验证

## 风险与约束

### 技术风险

- Tauri 与 Playwright 的集成和分发比普通 Web 项目复杂
- 内置浏览器运行时会显著增大安装包体积
- 模型容易生成“看起来合理但不可执行”的规则
- 浏览器自动化与 Reader-Rust 实际抓取行为可能仍存在差异

### 业务风险

- 部分站点禁止抓取或限制访问频率
- 站点频繁改版会让规则失效
- 登录态、验证码和反爬会显著拉低成功率

### 文档与实现一致性

本工具的输出必须始终以 Reader-Rust 当前实现为准。每次主项目的书源模型、规则语法或测试接口变更后，都应同步检查以下文档：

- [Reader-Rust 书源开发概述](../reader-rust/docs/book-source/index.md)
- [Reader-Rust 规则语法](../reader-rust/docs/book-source/rules.md)
- [Reader-Rust 书源管理 API](../reader-rust/docs/api/book-source.md)

## 总结

AI 书源设计工具值得做，但应明确做成一个本地运行的 Tauri 桌面应用，并把内置 Playwright、AI 配置管理和本地验证闭环作为核心能力。第一版重点应放在稳定站点、稳定规则和可复现调试，而不是追求覆盖任意站点，更不应过早引入向量数据库。

如果要启动开发，建议先完成桌面应用骨架、内置 Playwright 运行链路，再打通搜索、详情、目录、正文四段规则的最小闭环。
