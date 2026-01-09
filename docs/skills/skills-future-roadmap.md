# Skills 功能路线图

本文档记录基于当前 Plan Mode Skills 实现的功能状态和未来规划。

> **当前状态**：Skills 核心功能已完整实现，包括脚本执行、权限控制、资源限制等
>
> **参考文档**：
> - [plan-mode-skills-implementation.md](./plan-mode-skills-implementation.md) - 实现详情
> - [skills-architecture.md](./skills-architecture.md) - 架构设计
> - [skill-development-guide.md](./skill-development-guide.md) - 开发指南

- [Skills 功能路线图](#skills-功能路线图)
  - [一、功能概览](#一功能概览)
    - [1.1 已完成功能](#11-已完成功能)
      - [核心模块](#核心模块)
      - [脚本执行](#脚本执行)
      - [配置系统](#配置系统)
    - [1.2 API 与运行时管理](#12-api-与运行时管理)
    - [1.3 资源加载（已完成）](#13-资源加载已完成)
  - [二、已实现功能详情](#二已实现功能详情)
    - [2.1 脚本执行系统](#21-脚本执行系统)
    - [2.2 资源限制](#22-资源限制)
    - [2.3 权限控制](#23-权限控制)
    - [2.4 文件访问策略](#24-文件访问策略)
  - [三、资源加载功能](#三资源加载功能)
    - [3.1 当前支持](#31-当前支持)
    - [3.2 资源目录（已完成）](#32-资源目录已完成)
    - [3.3 实现任务](#33-实现任务)
      - [任务 3.3.1：References 自动注入 ✅](#任务-331references-自动注入-)
      - [任务 3.3.2：Assets 工具支持 ✅](#任务-332assets-工具支持-)
  - [四、多技能支持](#四多技能支持)
    - [4.1 功能描述](#41-功能描述)
    - [4.2 实现任务](#42-实现任务)
      - [任务 4.2.1：多技能检测 ✅](#任务-421多技能检测-)
      - [任务 4.2.2：多技能注入 ✅](#任务-422多技能注入-)
      - [任务 4.2.3：执行跟踪 ✅](#任务-423执行跟踪-)
      - [任务 4.2.4：多技能同时激活 ✅](#任务-424多技能同时激活-)
  - [五、技能管理 API](#五技能管理-api)
    - [5.1 API 设计](#51-api-设计)
    - [5.2 实现任务](#52-实现任务)
      - [任务 5.2.1：API 端点 ✅](#任务-521api-端点-)
      - [任务 5.2.2：权限控制 ✅](#任务-522权限控制-)
  - [六、高级特性](#六高级特性)
    - [6.1 技能市场/远程加载](#61-技能市场远程加载)
    - [6.2 技能版本管理](#62-技能版本管理)
    - [6.3 技能依赖](#63-技能依赖)
    - [6.4 技能模板](#64-技能模板)
  - [七、优先级矩阵](#七优先级矩阵)
    - [核心功能（已完成）](#核心功能已完成)
    - [待实现](#待实现)
    - [高级特性（分阶段）](#高级特性分阶段)
    - [其他高级特性](#其他高级特性)
  - [文档版本](#文档版本)

---

## 一、功能概览

### 1.1 已完成功能

#### 核心模块

| 功能 | 状态 | 位置 | 说明 |
| ---- | ---- | ---- | ---- |
| SKILL.md 解析 | ✅ 完成 | `parser.rs` | YAML 前置块 + Markdown |
| 技能注册表 | ✅ 完成 | `registry.rs` | 全局单例，支持热重载 |
| 两阶段加载 | ✅ 完成 | `plan.rs` | Phase 1 摘要 + Phase 2 完整内容 |
| 技能检测 | ✅ 完成 | `detector.rs` | `<use_skill>` 标签检测 |
| 提示词注入 | ✅ 完成 | `injector.rs` | 两阶段注入机制 |
| 工具过滤 | ✅ 完成 | `plan.rs` | `allowed-tools` 字段支持 |
| 执行跟踪 | ✅ 完成 | `trace.rs` | 完整执行记录 |
| 字段验证 | ✅ 完成 | `validator.rs` | name/description 格式验证 |

#### 脚本执行

| 功能 | 状态 | 位置 | 说明 |
| ---- | ---- | ---- | ---- |
| Deno 执行器 | ✅ 完成 | `executor/deno.rs` | JS/TS 脚本执行 |
| Docker 执行器 | ✅ 完成 | `executor/docker.rs` | Python/Shell/Ruby 执行 |
| 资源限制 | ✅ 完成 | `executor/types.rs` | 内存/超时/输出限制 |
| 脚本权限控制 | ✅ 完成 | `types.rs` | `allowed-scripts` 字段 |
| 文件访问策略 | ✅ 完成 | `executor/docker.rs` | bind/copy 双模式 |
| 内部工具 | ✅ 完成 | `plan.rs` | `internal__skill_run_script` |

#### 配置系统

| 功能 | 状态 | 位置 | 说明 |
| ---- | ---- | ---- | ---- |
| Skills 配置 | ✅ 完成 | `config.toml` | 目录、启用开关 |
| 执行器配置 | ✅ 完成 | `config.toml` | Deno/Docker 参数 |
| 资源限制配置 | ✅ 完成 | `config.toml` | 全局默认限制 |
| Docker 镜像映射 | ✅ 完成 | `config.toml` | 扩展名→镜像映射 |

### 1.2 API 与运行时管理

| 功能 | 状态 | 位置 | 说明 |
| ---- | ---- | ---- | ---- |
| 多技能注入 | ✅ 完成 | `injector.rs` | `multi_skill_injection()` 方法 |
| 技能启用/禁用 | ✅ 完成 | `handlers.rs` | `PUT /api/skills/{name}/enabled` |
| 技能重载 | ✅ 完成 | `handlers.rs` | `POST /api/skills/{name}/reload` |
| API 认证 | ✅ 完成 | `middleware.rs` | Bearer token 认证（可选） |
| 速率限制 | ✅ 完成 | `middleware.rs` | 滑动窗口算法 |

### 1.3 资源加载（已完成）

| 功能 | 状态 | 位置 | 说明 |
| ---- | ---- | ---- | ---- |
| References 自动注入 | ✅ 完成 | `injector.rs` | Phase 2 自动加载 `references/` 目录 |
| Assets 工具加载 | ✅ 完成 | `plan.rs` | `internal__skill_load_asset` 工具 |

---

## 二、已实现功能详情

### 2.1 脚本执行系统

```mermaid
graph TB
    subgraph 执行入口
        TOOL["internal__skill_run_script"]
    end

    subgraph 执行管理器
        MGR["ScriptExecutorManager"]
    end

    subgraph 执行器
        DENO["Deno 执行器<br/>.js .ts .mjs .tsx"]
        DOCKER["Docker 执行器<br/>.py .sh .rb"]
    end

    subgraph 输出
        OUT["ScriptOutput<br/>stdout/stderr/exit_code"]
    end

    TOOL --> MGR
    MGR --> DENO
    MGR --> DOCKER
    DENO --> OUT
    DOCKER --> OUT
```

**支持的脚本类型**：

| 执行器 | 扩展名 | 运行环境 |
| ------ | ------ | -------- |
| Deno | `.js`, `.ts`, `.mjs`, `.mts`, `.jsx`, `.tsx` | Deno Runtime |
| Docker | `.py` | `python:3.11-slim` |
| Docker | `.sh`, `.bash` | `alpine:latest` |
| Docker | `.rb` | `ruby:3.2-slim` |

### 2.2 资源限制

**三级优先级**（从高到低）：

1. 请求参数中指定的限制
2. Skill 级别的 `execution-limits`（通过 `metadata` 扩展字段）
3. 全局默认限制（config.toml）

```yaml
# SKILL.md 中的 Skill 级别限制（使用 metadata 扩展字段）
---
name: my-skill
description: A skill with custom resource limits
metadata:
  execution-limits: "max_memory_bytes=134217728,timeout_secs=30,network_access=true"
---
```

> **注意**：根据 [Agent Skills Standard](https://agentskills.io/specification#frontmatter-required)，
> 自定义扩展字段必须放在 `metadata` 字段中。`execution-limits` 使用键值对格式：
> - `max_memory_bytes=<bytes>` - 最大内存
> - `timeout_secs=<seconds>` - 超时时间
> - `max_output_bytes=<bytes>` - 最大输出大小
> - `network_access=true|false` - 是否允许网络访问

```toml
# config.toml 中的全局默认限制
[skill.execution.limits]
max_memory_bytes = 268435456    # 256MB
timeout = "30s"
max_output_bytes = 1048576      # 1MB
network_access = false
```

### 2.3 权限控制

**脚本执行权限**（通过 `metadata.allowed-scripts` 扩展字段）：

```yaml
metadata:
  allowed-scripts: "*.js, process.py, data-*.json"  # 逗号分隔的模式列表
```

> 支持的模式：
> - `*.js` - 通配符匹配
> - `process.py` - 精确匹配
> - `data-*.json` - 前缀匹配

**工具访问控制**（`allowed-tools` 字段，Agent Skills Standard 标准字段）：

```yaml
allowed-tools: Read Grep Bash(git:*) mcp__calc__sum
```

### 2.4 文件访问策略

**Docker 执行器的双重策略**：

| 模式 | 说明 | 适用场景 |
| ---- | ---- | -------- |
| `auto` | 优先 bind mount，失败则 copy | 默认，兼容性最好 |
| `bind_only` | 仅 bind mount | 高性能场景 |
| `copy_only` | 仅复制文件到临时目录 | 远程/网络文件系统 |

```toml
[skill.execution.docker]
file_access_mode = "auto"
data_dirs = ["/path/to/skills"]
```

---

## 三、资源加载功能

### 3.1 当前支持

```
skill-name/
├── SKILL.md           # ✅ 完全支持
└── scripts/           # ✅ 完全支持（可执行）
    ├── fetch-data.js
    └── process.py
```

### 3.2 资源目录（已完成）

```
skill-name/
├── references/        # ✅ 已完成自动注入
│   ├── api-docs.md
│   └── examples.txt
└── assets/            # ✅ 已完成工具加载
    ├── template.md
    └── config.json
```

| 资源目录 | 用途 | 当前状态 |
| -------- | ---- | -------- |
| `references/` | 参考文档注入到上下文 | ✅ Phase 2 自动注入 |
| `assets/` | 模板文件按需加载 | ✅ `internal__skill_load_asset` 工具 |

### 3.3 实现任务

#### 任务 3.3.1：References 自动注入 ✅

- [x] 修改 `phase2_injection()` 自动加载 `references/` 内容
- [x] 添加配置项控制最大参考文档大小（`max_reference_size`）
- [x] 支持 `references` 字段指定要加载的文件列表（支持 glob 模式）

**实现细节**：

- `SkillInjector::phase2_injection_auto_refs()` 自动加载并注入参考文档
- `SkillLoader::load_references_with_patterns()` 支持按模式过滤文件
- 配置项 `[skill].max_reference_size` 控制最大加载大小（默认 100KB）
- `references` 通过 `metadata` 扩展字段配置（符合 Agent Skills Standard）

**配置示例**：

```yaml
---
name: my-skill
description: A skill with custom references
metadata:
  references: "api-docs.md, *.txt"  # 逗号分隔的 glob 模式
---
```

#### 任务 3.3.2：Assets 工具支持 ✅

- [x] 添加 `internal__skill_load_asset` 工具供 LLM 调用
- [x] 实现模板变量替换功能（`{{variable}}` 语法）
- [x] 支持 JSON/YAML/Markdown 格式解析

**实现细节**：

- `internal__skill_load_asset` 内部工具允许 LLM 按需加载技能的 assets/ 文件
- 支持模板变量替换：使用 `{{key}}` 语法，通过 `variables` 参数传入键值对
- 支持三种格式解析：
  - `parse_as: "json"` - 解析并格式化 JSON 内容
  - `parse_as: "yaml"` - 解析并格式化 YAML 内容
  - `parse_as: "markdown"` - 以 Markdown 格式返回内容
- 工具参数：
  - `asset_name`（必需）：assets/ 目录下的文件名
  - `variables`（可选）：模板变量对象
  - `parse_as`（可选）：解析格式

**使用示例**：

```json
{
  "name": "internal__skill_load_asset",
  "arguments": {
    "asset_name": "report-template.md",
    "variables": {"title": "Monthly Report", "date": "2024-01-15"},
    "parse_as": "markdown"
  }
}
```

---

## 四、多技能支持

### 4.1 功能描述

当前设计仅支持单技能激活。未来可支持同时激活多个技能。

```mermaid
sequenceDiagram
    participant User
    participant Plan as Plan Mode
    participant Detector as SkillDetector
    participant Injector as SkillInjector

    User->>Plan: 复杂请求（需要多个 Skills）
    Plan->>Detector: detect_all() - 检测多个技能
    Detector-->>Plan: ["code-review", "doc-gen"]
    Plan->>Injector: multi_skill_injection(skills)
    Injector-->>Plan: 合并后的提示词
    Plan->>User: 执行结果
```

### 4.2 实现任务

#### 任务 4.2.1：多技能检测 ✅

- [x] 修改 `SkillDetector::detect()` 支持返回多个技能
- [x] 添加技能优先级/冲突解决机制
- [x] 更新 `<use_skill>` 标签格式支持多个（逗号分隔）

**实现细节**：

- `SkillDetector::detect()` 已支持检测多个技能，包括：
  - 多个 `<use_skill>` 标签：`<use_skill>a</use_skill> <use_skill>b</use_skill>`
  - 逗号分隔格式：`<use_skill>skill-a, skill-b, skill-c</use_skill>`
  - 自动去重：重复的技能名只保留一次
- `SkillMetadata` 通过 `metadata` 字段支持扩展（符合 Agent Skills Standard）：
  - `metadata.priority` - 技能优先级（字符串格式，-100 到 100，默认 0）
  - `metadata.conflicts` - 冲突的技能列表（逗号分隔的字符串）
- 新增优先级解析方法：
  - `SkillMetadata::get_priority()` - 从 metadata 中获取优先级
  - `SkillMetadata::get_conflicts()` - 从 metadata 中获取冲突列表
  - `SkillDetector::resolve_by_priority()` - 按优先级排序技能（高优先级在前）
- 新增冲突检测方法：
  - `SkillDetector::resolve_conflicts()` - 检测并移除冲突的低优先级技能
  - `SkillDetector::detect_and_resolve()` - 一站式方法：检测 + 优先级排序 + 冲突解决

**SKILL.md 配置示例**：

```yaml
---
name: primary-skill
description: High priority skill
metadata:
  priority: "20"
  conflicts: "secondary-skill, legacy-skill"
---
```

#### 任务 4.2.2：多技能注入 ✅

- [x] 启用 `SkillInjector::multi_skill_injection()`
- [x] 合并多个技能的 `allowed-tools`（取并集）
- [x] 合并多个技能的 `allowed-scripts`

**实现细节**：

- `SkillInjector::multi_skill_injection()` 已完全重构，支持多技能合并注入
- 输出格式包含：
  - 技能名称列表头部：`## Active Skills: skill-a, skill-b`
  - 合并权限区块：显示所有技能的 allowed-tools 和 allowed-scripts 并集
  - 各技能内容区块：每个技能单独一个 `### Skill: name` 区块
- 新增合并辅助方法：
  - `SkillInjector::merge_allowed_tools()` - 合并多个技能的 allowed-tools（去重保序）
  - `SkillInjector::merge_allowed_scripts()` - 合并多个技能的 allowed-scripts（去重保序）
- 新增引用支持方法：
  - `SkillInjector::multi_skill_injection_with_refs()` - 带手动引用注入
  - `SkillInjector::multi_skill_injection_auto_refs()` - 自动加载各技能的 references/ 并注入

**多技能注入输出示例**：

```markdown
## Active Skills: code-review, security-scan

### Merged Permissions

**Allowed Tools:** Bash, Read, Grep, WebFetch

**Allowed Scripts:** *.js, *.py, scan-*.sh

---

### Skill: code-review

[code-review 技能的完整内容]

---

### Skill: security-scan

[security-scan 技能的完整内容]

---
```

#### 任务 4.2.3：执行跟踪 ✅

- [x] 修改 `SubtaskTrace::active_skill` 为 `Vec<String>`
  - 重命名为 `active_skills: Vec<String>`
  - 添加 `add_active_skill()` 方法（去重）
  - 添加 `set_active_skills()` 方法（替换）
  - 添加 `has_active_skills()` 和 `get_active_skills()` 辅助方法
- [x] 更新跟踪日志格式
  - 单技能：`skill=name`
  - 多技能：`skills=[name1, name2]`
- [x] 添加多技能跟踪测试（13 个新测试）

#### 任务 4.2.4：多技能同时激活 ✅

> **目标**：在 `plan.rs` 中实现多技能同时激活，完成从检测到执行的完整多技能支持链路。

- [x] 修改 `Plan` 结构中的技能字段
  - 将 `active_skill: Option<LoadedSkill>` 改为 `active_skills: Vec<LoadedSkill>`
  - 更新相关的 getter/setter 方法
- [x] 集成多技能检测
  - 使用 `SkillDetector::detect_and_resolve()` 进行多技能检测与冲突解决
  - 支持逗号分隔的技能激活（如 `<use_skill>code-review, security-scan</use_skill>`）
- [x] 集成多技能注入
  - 使用 `SkillInjector::multi_skill_injection_auto_refs()` 合并多技能内容
  - 正确处理优先级和资源限制
- [x] 使用 `SubtaskTrace` 的多技能跟踪方法
  - `set_active_skills()` - 设置激活的技能列表
  - `has_active_skills()` - 检查是否有激活的技能
  - `get_active_skills()` - 获取激活的技能列表
- [x] 添加集成测试（3 个新测试）
  - `test_integration_multi_skill_context` - 测试多技能上下文构建
  - `test_integration_multi_skill_tool_filtering` - 测试多技能工具过滤（合并 allowed_tools）
  - `test_integration_multi_skill_tracing` - 测试多技能跟踪

**实现细节**：

- `execute_subtask_with_react()` 函数重构：
  - `active_skill: Option<LoadedSkill>` → `active_skills: Vec<LoadedSkill>`
  - 技能检测使用 `SkillDetector::detect_and_resolve()` 进行优先级排序和冲突解决
  - 支持 `<use_skill>skill-a, skill-b</use_skill>` 逗号分隔格式
- `filter_tools_by_skills()` 函数重构：
  - Phase 2 使用 `SkillInjector::merge_allowed_tools()` 合并多技能的 allowed_tools
  - 工具过滤使用合并后的并集
- `build_context_for_react()` 函数重构：
  - Phase 2 使用 `SkillInjector::multi_skill_injection_auto_refs()` 合并多技能内容
  - 系统提示显示 `Active Skills: skill-a, skill-b` 格式
- `SkillRegistry::get_all_loaded()` 新增方法：
  - 返回所有已加载且启用的技能，用于优先级/冲突解决

> **依赖**：任务 4.2.1（多技能检测）、任务 4.2.2（多技能注入）、任务 4.2.3（执行跟踪）

---

## 五、技能管理 API

### 5.1 API 设计

| 端点 | 方法 | 功能 | 状态 |
| ---- | ---- | ---- | ---- |
| `/api/skills` | GET | 列出所有技能摘要 | ✅ 已完成 |
| `/api/skills/{name}` | GET | 获取技能详情 | ✅ 已完成 |
| `/api/skills/{name}/enabled` | PUT | 启用/禁用技能 | ✅ 已完成 |
| `/api/skills/{name}/reload` | POST | 重载指定技能 | ✅ 已完成 |
| `/api/skills/reload` | POST | 重载所有技能 | ✅ 已完成 |

> **注意**：底层方法 `registry.set_enabled()` 和 `registry.reload()` 已实现

### 5.2 实现任务

#### 任务 5.2.1：API 端点 ✅

- [x] 创建 `src/skills/handlers.rs`
- [x] 实现 CRUD 操作
- [x] 添加路由配置到 `main.rs`

**实现细节**：

- `src/skills/handlers.rs` 实现了 5 个 HTTP 处理程序：
  - `list_skills_handler()` - 列出所有启用的技能摘要
  - `get_skill_handler()` - 获取技能详情（包含 content、scripts 等）
  - `set_skill_enabled_handler()` - 启用/禁用技能
  - `reload_skill_handler()` - 重载单个技能
  - `reload_all_skills_handler()` - 重载所有技能
- 路由仅在 Skills 系统初始化后启用（Plan Mode 且 `skill.enabled = true`）
- 所有端点返回 JSON 响应，包含适当的错误处理
- 使用 `x-request-id` 头进行请求跟踪

**响应格式**：

```json
// GET /api/skills
{
  "skills": [
    { "name": "code-review", "description": "...", "allowed_tools": [...] }
  ],
  "total": 1
}

// GET /api/skills/{name}
{
  "name": "code-review",
  "description": "...",
  "enabled": true,
  "license": "MIT",
  "allowed_tools": [...],
  "allowed_scripts": ["*.js"],
  "scripts": ["analyze.js"],
  "content": "# Code Review..."
}

// PUT /api/skills/{name}/enabled, POST /api/skills/{name}/reload
{
  "success": true,
  "message": "Skill 'code-review' enabled successfully",
  "skill_name": "code-review"
}

// POST /api/skills/reload
{
  "success": true,
  "message": "Reloaded 3 skills successfully",
  "skills_loaded": 3
}
```

#### 任务 5.2.2：权限控制 ✅

- [x] 添加 API 认证（可选）
- [x] 实现速率限制

**实现细节**：

- `src/skills/middleware.rs` 实现了认证和速率限制中间件：
  - `skills_api_middleware()` - 处理认证和速率限制检查
  - `RateLimiter` - 滑动窗口速率限制器
  - `SkillsApiState` - 中间件状态管理
- `src/config.rs` 新增 `SkillApiConfig` 配置结构

**配置示例**：

```toml
[skill.api]
# API 密钥认证（可选，留空则不需要认证）
# 也可通过 SKILLS_API_KEY 环境变量设置
api_key = "sk-your-secret-key"

# 速率限制：每个时间窗口内允许的最大请求数
# 设置为 0 禁用速率限制，默认：100
rate_limit_requests = 100

# 速率限制时间窗口（秒），默认：60
rate_limit_window_secs = 60
```

**认证方式**：

请求需在 `Authorization` 头中携带 API 密钥：

```http
Authorization: Bearer sk-your-secret-key
```

**速率限制响应**：

当超过速率限制时，返回 HTTP 429：

```json
{
  "error": "Rate limit exceeded"
}
```

---

## 六、高级特性

### 6.1 技能市场/远程加载

#### 6.1.1 功能意义

远程技能加载功能旨在解决以下问题：

1. **技能共享与复用**：允许开发者将通用技能发布到公共仓库，其他用户可以直接安装使用，避免重复开发。

2. **版本管理**：通过远程仓库管理技能版本，用户可以选择安装特定版本或自动更新到最新版本。

3. **团队协作**：企业或团队可以维护内部技能仓库，统一管理和分发团队专用技能。

4. **生态建设**：构建技能市场生态，促进 AI Agent 技能的标准化和规范化发展。

```mermaid
graph TB
    subgraph Developers["技能开发者"]
        D1[个人开发者]
        D2[企业团队]
        D3[开源社区]
    end

    subgraph Market["技能市场"]
        M1[skillsmp.com]
        M2[企业私有市场]
    end

    subgraph Sources["其他来源"]
        S1[GitHub/GitLab]
        S2[私有注册中心]
    end

    subgraph Users["技能使用者"]
        U1[本地安装]
        U2[按需加载]
    end

    D1 -->|发布| M1
    D2 -->|发布| M2
    D3 -->|发布| M1
    D1 -->|开源| S1
    D2 -->|内部| S2

    M1 -->|安装| U1
    M2 -->|安装| U1
    S1 -->|安装| U1
    S2 -->|安装| U1
```

#### 6.1.2 技能来源

```mermaid
graph LR
    subgraph Local["本地技能"]
        L1[.skills/]
        L2[~/.llama-nexus/skills/]
    end

    subgraph Market["技能市场"]
        M1[skillsmp.com]
    end

    subgraph Git["Git 仓库"]
        G1[GitHub]
        G2[GitLab/Gitee]
    end

    subgraph Private["私有源"]
        P1[企业注册中心]
    end

    M1 -->|API 下载| L2
    G1 -->|git clone| L1
    G2 -->|git clone| L1
    P1 -->|认证访问| L2
```

远程技能支持多种来源：

| 来源类型 | URL 格式 | 说明 |
| -------- | -------- | ---- |
| 技能市场 | `skillsmp:skill-name` 或 `https://skillsmp.com/skills/name` | 公共技能市场，如 skillsmp.com |
| GitHub | `github:owner/repo` 或 `https://github.com/owner/repo` | 最常用的公开技能托管平台 |
| GitLab | `gitlab:owner/repo` 或 `https://gitlab.com/owner/repo` | 支持私有仓库 |
| Gitee | `gitee:owner/repo` | 国内镜像，访问更快 |
| HTTP(S) | `https://example.com/skills/my-skill.tar.gz` | 直接下载压缩包 |
| 私有注册中心 | `registry:skill-name@version` | 企业私有技能仓库 |

##### skillsmp.com API 集成

[skillsmp.com](https://skillsmp.com/) 是一个聚合 GitHub 上 Agent Skills 的公共市场，提供 30,000+ 技能供搜索和下载。

###### 技能 URL 格式

技能页面 URL 格式为：

```text
https://skillsmp.com/skills/{skill-id}
```

其中 `skill-id` 基于 GitHub 路径生成，格式为 `{owner}-{repo}-{path-to-skill-md}`。

示例：

- `https://skillsmp.com/skills/krmcbride-claude-plugins-essentials-skills-documentation-lookup-skill-md`
- `https://skillsmp.com/skills/openai-codex-codex-rs-core-src-skills-assets-samples-skill-installer-skill-md`

###### API 端点

```bash
# 搜索技能（AI 语义搜索）
curl -X GET "https://skillsmp.com/api/v1/skills/ai-search?q=code+review" \
  -H "Authorization: Bearer sk_live_your_api_key"

# 获取技能详情
curl -X GET "https://skillsmp.com/api/v1/skills/{skill-id}" \
  -H "Authorization: Bearer sk_live_your_api_key"

# 下载技能包（zip 格式）
curl -X GET "https://skillsmp.com/api/v1/skills/{skill-id}/download" \
  -H "Authorization: Bearer sk_live_your_api_key" \
  -o skill.zip
```

###### 网页下载

在技能页面提供 `wget skill.zip` 按钮，可下载包含 SKILL.md 和所有相关文件的完整技能目录。

###### 认证方式

- 免费技能：无需认证
- 付费/私有技能：需要 API Key（`sk_live_xxx` 格式）

#### 6.1.3 使用方法

##### 安装远程技能

```bash
# 从技能市场安装（推荐）✅ 已实现
llama-nexus skill install skillsmp:code-review
llama-nexus skill install skillsmp:code-review@2.0.0

# 搜索技能市场 ✅ 已实现
llama-nexus skill search "code review"
llama-nexus skill search "code review" --category development

# 从 HTTP URL 安装 ✅ 已实现
llama-nexus skill install https://example.com/skills/my-skill.zip

# 安装到指定目录 ✅ 已实现
llama-nexus skill install skillsmp:code-review --dir ~/.llama-nexus/skills/
llama-nexus skill install skillsmp:code-review -d /custom/path

# 安装并自动启用 ✅ 已实现（标志存在，功能待完善）
llama-nexus skill install skillsmp:code-review --enable

# 从 GitHub 安装（阶段三）🔮 规划中
llama-nexus skill install github:user/awesome-skill
llama-nexus skill install github:user/awesome-skill@v1.2.0

# 从私有注册中心安装（阶段三）🔮 规划中
llama-nexus skill install registry:code-review@latest
```

##### 管理已安装技能

```bash
# 列出本地已安装技能 ✅ 已实现
llama-nexus skill list

# 列出远程热门技能 ✅ 已实现
llama-nexus skill list --remote
llama-nexus skill list -r -n 20

# 检查技能版本信息 ✅ 已实现
llama-nexus skill outdated

# 更新单个技能 ✅ 已实现
llama-nexus skill update code-review

# 更新所有技能 ✅ 已实现
llama-nexus skill update --all
llama-nexus skill update -a

# 卸载技能 ✅ 已实现
llama-nexus skill uninstall code-review
llama-nexus skill uninstall code-review --yes  # 跳过确认

# 查看本地技能详情 ✅ 已实现
llama-nexus skill info code-review

# 查看远程技能详情 ✅ 已实现
llama-nexus skill info skillsmp:code-review
```

##### 配置技能源

```toml
# config.toml

# 技能市场配置 ✅ 已实现
[skill.market]
# API URL（默认：https://skillsmp.com/api/v1）
# url = "https://skillsmp.com/api/v1"

# API 密钥（或设置 SKILLSMP_API_KEY 环境变量）
# api_key = "sk_live_your_api_key"

# 下载缓存目录（可选）
# cache_dir = "~/.llama-nexus/cache/skills"

# 私有注册中心（阶段三）🔮 规划中
# [[skill.registry.sources]]
# name = "company"
# url = "https://skills.company.com/api/v1"
# token_env = "COMPANY_SKILLS_TOKEN"

# GitHub 镜像加速（阶段三）🔮 规划中
# [skill.registry.github]
# mirror = "https://ghproxy.com/"

# 缓存配置（阶段四）🔮 规划中
# [skill.cache]
# enabled = true
# dir = "~/.llama-nexus/cache/skills"
# ttl_hours = 24
```

#### 6.1.4 技能包格式

远程技能使用标准目录结构打包：

```text
my-skill/
├── SKILL.md           # 必需：技能定义文件
├── scripts/           # 可选：可执行脚本目录
│   ├── process.js
│   └── analyze.py
├── references/        # 可选：参考文档目录（Phase 2 自动注入）
│   ├── api-docs.md
│   └── examples.txt
├── assets/            # 可选：资源文件目录（按需加载）
│   ├── template.md
│   └── config.json
├── LICENSE            # 推荐：许可证文件
├── README.md          # 推荐：使用说明
└── skill.lock         # 自动生成：版本锁定文件 ✅ 已实现
```

##### skill.lock 文件 ✅ 已实现

安装远程技能时自动生成，用于版本跟踪和更新管理：

```yaml
# skill.lock - Auto-generated, do not edit manually
# https://github.com/secondstate/llama-nexus

name: code-review
version: 2.1.0                              # 可选：版本号
source: skillsmp:code-review@2.1.0          # 安装来源
installed_at: 2024-01-15T10:30:00+00:00     # ISO 8601 时间戳
checksum: ~                                 # 预留：完整性校验（阶段四）
```

> **当前实现**：`skill.lock` 文件支持 `name`、`version`、`source`、`installed_at` 字段。
> `checksum` 和 `dependencies` 字段为阶段四规划功能。

#### 6.1.5 安全机制

```mermaid
graph TD
    subgraph Install["安装流程"]
        I1[下载技能包]
        I2[验证签名]
        I3[检查权限声明]
        I4[沙箱扫描]
        I5[用户确认]
        I6[安装完成]
    end

    I1 --> I2
    I2 -->|验证通过| I3
    I2 -->|验证失败| X1[拒绝安装]
    I3 --> I4
    I4 -->|无风险| I6
    I4 -->|有风险| I5
    I5 -->|用户同意| I6
    I5 -->|用户拒绝| X2[取消安装]
```

##### 签名验证

技能发布者可以使用 GPG 或类似机制对技能进行签名：

```bash
# 发布者：签名技能
llama-nexus skill sign my-skill --key ~/.gnupg/my-key.asc

# 使用者：验证签名
llama-nexus skill verify my-skill

# 配置信任的签名者
# config.toml
[skills.security]
require_signature = true  # 是否强制要求签名
trusted_keys = [
    "ABC123...",  # 信任的公钥指纹
    "DEF456...",
]
```

##### 权限审查

安装时自动检查技能声明的权限：

```text
Installing skill: code-review@2.1.0

Permissions requested:
  ✓ Read: Read files from workspace
  ✓ Bash: Execute git commands
  ⚠ Write: Write to current directory (requires confirmation)

Scripts included:
  - analyze.js (Deno sandbox)
  - format.py (Docker container)

Resource limits:
  - Memory: 128MB
  - Timeout: 30s
  - Network: disabled

Continue installation? [y/N]
```

#### 6.1.6 分阶段实现计划

##### 阶段一：skillsmp.com 基础集成（MVP）✅

目标：通过命令行从 skillsmp.com 安装技能

- [x] **任务 P1.1**：实现 CLI 子命令框架
  - 添加 `llama-nexus skill` 子命令入口
  - 实现基础参数解析（install, list, info）
  - 实现位置：`src/cli/mod.rs`, `src/cli/skill.rs`

- [x] **任务 P1.2**：实现 skillsmp.com 下载
  - 调用 `GET /api/v1/skills/{skill-id}/download` 获取 zip
  - 解压到 `~/.llama-nexus/skills/` 目录
  - 支持 `skill install skillsmp:skill-name` 语法
  - 实现位置：`src/cli/skill/installer.rs`, `src/cli/skill/marketplace.rs`

- [x] **任务 P1.3**：实现本地技能列表
  - `skill list` 显示已安装技能
  - `skill list --remote` 从 skillsmp.com 获取热门技能
  - 注意：远程列表需要 API key（Cloudflare 保护）

- [x] **任务 P1.4**：实现基础技能信息
  - `skill info <name>` 显示本地技能详情
  - `skill info skillsmp:<name>` 显示市场技能详情

##### 阶段二：完整技能管理 ✅

目标：完善技能生命周期管理

- [x] **任务 P2.1**：实现技能搜索
  - `skill search <query>` 搜索 skillsmp.com
  - 支持分类过滤 `--category`
  - 实现位置：`src/cli/skill.rs` (`search_skills()`)

- [x] **任务 P2.2**：实现技能更新
  - `skill update <name>` 更新单个技能
  - `skill update --all` 更新所有技能
  - `skill outdated` 检查可更新技能
  - 实现位置：`src/cli/skill.rs` (`update_skills()`, `check_outdated_skills()`)

- [x] **任务 P2.3**：实现技能卸载
  - `skill uninstall <name>` 删除技能
  - 支持 `--yes` 跳过确认提示
  - 实现位置：`src/cli/skill.rs` (`uninstall_skill()`)

- [x] **任务 P2.4**：实现版本管理
  - 支持 `skill install skillsmp:name@version` 指定版本
  - 安装时自动生成 `skill.lock` 锁定文件
  - 实现位置：`src/cli/skill/lockfile.rs`, `src/cli/skill/installer.rs`

##### 阶段三：多源支持

目标：支持 GitHub 等其他技能来源

- [ ] **任务 P3.1**：实现 GitHub 安装
  - `skill install github:owner/repo`
  - 支持分支/标签 `@v1.0.0` 或 `#branch`
  - 支持子目录 `github:owner/repo/path/to/skill`

- [ ] **任务 P3.2**：实现 HTTP(S) 安装
  - `skill install https://example.com/skill.zip`
  - 支持 tar.gz 和 zip 格式

- [ ] **任务 P3.3**：实现私有注册中心
  - `skill install registry:name@version`
  - 支持企业私有仓库认证
  - 配置文件中定义注册中心

##### 阶段四：安全与高级功能

目标：增强安全性和用户体验

- [ ] **任务 P4.1**：实现安全机制
  - GPG 签名验证
  - 权限声明审查（安装前提示）
  - 可选的沙箱扫描

- [ ] **任务 P4.2**：实现缓存和镜像
  - 本地下载缓存
  - 镜像源配置（国内加速）
  - 离线安装支持

- [ ] **任务 P4.3**：实现配置管理
  - `skill config` 管理配置
  - API Key 安全存储
  - 默认安装目录设置

### 6.2 技能版本管理

```yaml
# SKILL.md
---
name: code-review
version: 2.0.0
min-agent-version: 0.9.0
---
```

**任务**：

- [ ] 添加 `version` 字段支持
- [ ] 实现版本兼容性检查
- [ ] 支持多版本共存

### 6.3 技能依赖

```yaml
# SKILL.md
---
name: full-review
dependencies:
  - code-review
  - security-scan
---
```

**任务**：

- [ ] 添加 `dependencies` 字段支持
- [ ] 实现依赖解析和加载顺序
- [ ] 处理循环依赖

### 6.4 技能模板

```bash
# 创建新技能
llama-nexus skill new my-skill --template basic
```

**任务**：

- [ ] 设计技能模板格式
- [ ] 实现 CLI 命令 `skill new`
- [ ] 提供内置模板（basic, tool-based, workflow）

---

## 七、优先级矩阵

### 核心功能（已完成）

| 功能 | 优先级 | 复杂度 | 依赖 | 状态 |
| ---- | ------ | ------ | ---- | ---- |
| 两阶段加载 | P0 | 高 | 无 | ✅ 完成 |
| 脚本执行 | P0 | 高 | 执行器 | ✅ 完成 |
| 权限控制 | P0 | 中 | 无 | ✅ 完成 |
| 资源限制 | P0 | 中 | 无 | ✅ 完成 |
| References 自动注入 | P1 | 低 | 无 | ✅ 完成 |
| Assets 工具加载 | P1 | 中 | 无 | ✅ 完成 |

### 待实现

| 功能 | 优先级 | 复杂度 | 依赖 | 状态 |
| ---- | ------ | ------ | ---- | ---- |
| 技能管理 API | P2 | 低 | 无 | ✅ 完成 |
| 多技能检测 | P3 | 中 | 无 | ✅ 完成 |
| 多技能注入 | P3 | 中 | 多技能检测 | ✅ 完成 |
| 多技能同时激活 | P3 | 中 | 多技能检测、多技能注入 | ✅ 完成 |

### 高级特性（分阶段）

| 功能 | 阶段 | 优先级 | 复杂度 | 状态 |
| ---- | ---- | ------ | ------ | ---- |
| skillsmp.com 基础安装 | P1 | P2 | 中 | ✅ 完成 |
| 技能列表和信息 | P1 | P2 | 低 | ✅ 完成 |
| 技能搜索 | P2 | P3 | 中 | ✅ 完成 |
| 技能更新/卸载 | P2 | P3 | 中 | ✅ 完成 |
| 版本管理 (skill.lock) | P2 | P3 | 中 | ✅ 完成 |
| GitHub 安装支持 | P3 | P3 | 中 | 🔮 规划中 |
| 私有注册中心 | P3 | P4 | 高 | 🔮 规划中 |
| 安全机制（签名验证） | P4 | P4 | 高 | 🔮 规划中 |
| 缓存和镜像 | P4 | P4 | 中 | 🔮 规划中 |

### 其他高级特性

| 功能 | 优先级 | 复杂度 | 依赖 | 状态 |
| ---- | ------ | ------ | ---- | ---- |
| 技能版本管理 | P4 | 中 | 无 | 🔮 规划中 |
| 技能依赖 | P4 | 高 | 版本管理 | 🔮 规划中 |
| 技能模板 | P4 | 低 | CLI 工具 | 🔮 规划中 |

---

## 文档版本

- **版本**: 2.9
- **创建日期**: 2024-12-31
- **更新日期**: 2026-01-09
- **适用项目版本**: llama-nexus v0.8.2 (feat-sandbox)

### 更新记录

**v2.9 (2026-01-09)**

- 完成阶段二（完整技能管理）所有任务
- 实现技能搜索：`skill search <query>` 支持 `--category` 过滤
- 实现技能更新：`skill update <name>` 和 `skill update --all`
- 实现技能卸载：`skill uninstall <name>` 支持 `--yes` 跳过确认
- 实现版本管理：安装时自动生成 `skill.lock` 文件用于版本跟踪
- 新增文件：`src/cli/skill/lockfile.rs`

**v2.8 (2026-01-09)**

- 完成阶段一（MVP）所有任务
- 实现 CLI 子命令框架：`llama-nexus skill {install,list,info}`
- 实现 skillsmp.com 下载集成（需要 API key 因 Cloudflare 保护）
- 实现本地技能列表和详情查看
- 新增文件：`src/cli/mod.rs`, `src/cli/skill.rs`, `src/cli/skill/installer.rs`, `src/cli/skill/marketplace.rs`
- 新增配置：`SkillMarketConfig` 支持市场 API key 配置

**v2.7 (2026-01-09)**

- 将 6.1 节实现任务重构为四阶段计划
- 阶段一（MVP）：skillsmp.com 基础安装功能
- 阶段二：完整技能管理（搜索、更新、卸载）
- 阶段三：多源支持（GitHub、HTTP、私有注册中心）
- 阶段四：安全与高级功能
- 更新优先级矩阵，按阶段展示

**v2.6 (2026-01-09)**

- 新增技能市场（skillsmp.com）集成支持
- 新增 skillsmp.com API 集成说明
- 新增 `skillsmp:skill-name` 协议和搜索功能
- 更新图表和配置示例，突出技能市场作为推荐安装方式

**v2.5 (2026-01-09)**

- 扩展 6.1 节"技能市场/远程加载"功能文档
- 新增功能意义说明（6.1.1）
- 新增技能来源类型表（6.1.2）
- 新增详细使用方法和命令示例（6.1.3）
- 新增技能包格式和 skill.lock 文件说明（6.1.4）
- 新增安全机制说明：签名验证、权限审查（6.1.5）
- 细化实现任务列表（6.1.6）

**v2.4 (2026-01-09)**
- 将 `execution-limits`、`allowed-scripts`、`references` 扩展字段迁移到 `metadata` 中
- 符合 Agent Skills Standard 对 frontmatter 的要求
- 扩展字段使用逗号分隔的字符串格式存储在 metadata 中
