# LlamaNexus Memory 配置指南

本文档详细介绍了 LlamaNexus 中 Memory 功能的各项配置参数，包括使用方法、注意事项和最佳实践。

- [LlamaNexus Memory 配置指南](#llamanexus-memory-配置指南)
  - [配置概览](#配置概览)
  - [详细配置说明](#详细配置说明)
    - [1. enable](#1-enable)
    - [2. database\_path](#2-database_path)
    - [3. context\_window](#3-context_window)
    - [4. auto\_summarize](#4-auto_summarize)
    - [5. summary\_service\_base\_url](#5-summary_service_base_url)
    - [6. summary\_service\_api\_key](#6-summary_service_api_key)
    - [7. max\_stored\_messages](#7-max_stored_messages)
    - [8. summarize\_threshold](#8-summarize_threshold)
  - [配置关系图](#配置关系图)
  - [最佳实践](#最佳实践)
    - [1. 参数配置建议](#1-参数配置建议)
    - [2. 性能优化](#2-性能优化)
    - [3. 故障排除](#3-故障排除)
  - [总结](#总结)

## 配置概览

Memory 功能通过智能的消息管理和自动总结机制，帮助系统在长对话中保持上下文连贯性，同时控制内存使用。

```toml
[memory]
enable = true
database_path = "data/memory.db"
context_window = 8192
auto_summarize = true
summary_service_base_url = "http://localhost:10086/v1"
summary_service_api_key = ""
max_stored_messages = 20
summarize_threshold = 12
```

## 详细配置说明

### 1. enable

**功能**：启用或禁用整个 Memory 功能。

**配置**：

```toml
enable = true  # 启用 Memory 功能
enable = false # 禁用 Memory 功能
```

**使用方法**：

- `true`：启用完整的 Memory 功能，包括消息存储、检索和自动总结
- `false`：禁用 Memory 功能，系统将不会存储历史对话

**注意事项**：

- 禁用后，所有相关的 memory 配置项都会失效
- 建议在生产环境中启用以提供更好的对话体验
- 禁用可以节省存储空间和计算资源

---

### 2. database_path

**功能**：指定 SQLite 数据库文件的存储路径。

**配置**：

```toml
database_path = "data/memory.db"
```

**使用方法**：

- 支持相对路径和绝对路径
- 相对路径是相对于应用程序的工作目录
- 文件扩展名通常使用 `.db`

**注意事项**：

- 确保指定的目录存在且具有写权限
- 建议定期备份数据库文件
- 数据库文件包含敏感的对话历史，注意文件安全
- 路径中不应包含特殊字符或空格

**目录结构示例**：

```bash
project/
├── data/
│   └── memory.db
├── config.toml
└── ...
```

---

### 3. context_window

**功能**：定义对话的最大上下文窗口大小（以 token 数量为单位）。

**配置**：

```toml
context_window = 8192  # 8K tokens
```

**使用方法**：

- 控制系统在处理对话时能够"记住"的最大上下文长度
- 当对话超过此限制时，会触发自动总结机制
- 数值越大，能保持的上下文越长

**推荐配置**：

- **小型应用**：2048-4096 tokens
- **中型应用**：4096-8192 tokens
- **大型应用**：8192-16384 tokens

**注意事项**：

- 必须考虑目标 LLM 的实际上下文限制
- 较大的值会消耗更多内存和计算资源
- 应该大于 `max_stored_messages` 对应的平均 token 数量
- 建议根据实际使用情况监控和调整

---

### 4. auto_summarize

**功能**：启用或禁用自动消息总结功能。

**配置**：

```toml
auto_summarize = true  # 启用自动总结
auto_summarize = false # 禁用自动总结
```

**使用方法**：

- `true`：当消息数量达到阈值时自动触发总结
- `false`：禁用自动总结，可能导致上下文超限

**注意事项**：

- 启用时需要配置 `summary_service_base_url`
- 禁用可能导致长对话中的上下文丢失
- 总结服务的质量直接影响对话连贯性
- 建议在生产环境中启用

---

### 5. summary_service_base_url

**功能**：指定用于消息总结的外部服务的基础 URL。

**配置**：

```toml
summary_service_base_url = "http://localhost:10086/v1"
```

**使用方法**：

- 提供完整的 HTTP/HTTPS URL
- 通常指向兼容 OpenAI API 格式的服务
- 支持本地部署和云服务

**URL 格式示例**：

```toml
# 本地服务
summary_service_base_url = "http://localhost:10086/v1"

# 远程服务
summary_service_base_url = "https://api.openai.com/v1"

# 自定义服务
summary_service_base_url = "https://your-custom-llm-service.com/v1"
```

**注意事项**：

- 确保服务可达性和稳定性
- 验证服务的 API 兼容性
- 考虑网络延迟对总结速度的影响
- 生产环境建议使用 HTTPS

---

### 6. summary_service_api_key

**功能**：用于访问总结服务的 API 密钥。

**配置**：

```toml
summary_service_api_key = ""          # 无需 API 密钥
summary_service_api_key = "sk-xxx..."  # OpenAI 格式密钥
```

**使用方法**：

- 空字符串表示无需身份验证
- 通常用于云服务或需要认证的 API
- 支持各种格式的 API 密钥

**注意事项**：

- **安全性**：密钥包含敏感信息，注意保护
- **环境变量**：建议通过环境变量传递密钥
- **权限控制**：确保密钥具有适当的权限
- **定期轮换**：定期更新 API 密钥

**安全最佳实践**：

```bash
# 使用环境变量
export SUMMARY_API_KEY="your-api-key"
```

---

### 7. max_stored_messages

**功能**：设置触发自动总结的消息数量阈值。

**配置**：

```toml
max_stored_messages = 20  # 20条消息时触发总结
```

**使用方法**：

- 当存储的消息数量达到此值时，自动触发总结流程
- 较小的值会频繁触发总结，较大的值可能导致上下文过长
- 需要与 `summarize_threshold` 配合使用

**推荐配置**：

- **短对话场景**：10-15 条消息
- **中等对话场景**：15-25 条消息
- **长对话场景**：25-40 条消息

**注意事项**：

- 必须大于 `summarize_threshold`
- 考虑消息的平均长度
- 频繁总结会增加计算开销
- 应该根据实际使用模式调整

---

### 8. summarize_threshold

**功能**：定义总结后保留的最近消息数量的计算基数。

**配置**：

```toml
summarize_threshold = 12  # 保留 threshold/2 = 6 条最近消息
```

**使用方法**：

- 总结后保留的消息数量 = `summarize_threshold / 2`
- 较老的消息会被总结为简洁的摘要
- 确保重要的近期上下文得以保留

**工作流程示例**：

```txt
初始状态：20条消息（达到 max_stored_messages）
↓
触发总结
↓
保留最近 6条消息（summarize_threshold/2 = 12/2 = 6）
总结前面 14条消息为摘要
↓
最终状态：1条摘要 + 6条原始消息
```

**注意事项**：

- 必须小于 `max_stored_messages`
- 建议 `max_stored_messages` 是 `summarize_threshold` 的 1.5-2 倍
- 保留的消息数量直接影响上下文连贯性
- 总结的消息数量影响摘要的详细程度

## 配置关系图

```txt
max_stored_messages (20) > summarize_threshold (12)
                              ↓
                        kept_messages = 12/2 = 6
                              ↓
                    summarized_messages = 20 - 6 = 14
```

## 最佳实践

### 1. 参数配置建议

```toml
# 平衡配置（推荐）
[memory]
enable = true
context_window = 8192
max_stored_messages = 20
summarize_threshold = 12
auto_summarize = true

# 高频对话配置
[memory]
enable = true
context_window = 4096
max_stored_messages = 15
summarize_threshold = 10
auto_summarize = true

# 长对话配置
[memory]
enable = true
context_window = 16384
max_stored_messages = 30
summarize_threshold = 18
auto_summarize = true
```

### 2. 性能优化

- **监控指标**：
  - 平均对话长度
  - 总结触发频率
  - 内存使用量
  - 响应时间

- **调优策略**：
  - 根据实际对话模式调整阈值
  - 监控总结服务的性能
  - 定期清理过期数据
  - 优化数据库查询

### 3. 故障排除

**常见问题**：

1. **总结服务不可用**
   - 检查 `summary_service_base_url` 的可达性
   - 验证 API 密钥的有效性
   - 考虑设置备用总结服务

2. **内存使用过高**
   - 减小 `context_window` 值
   - 降低 `max_stored_messages`
   - 增加总结频率

3. **对话上下文丢失**
   - 增加 `summarize_threshold` 值
   - 检查总结服务的质量
   - 考虑调整总结提示词

4. **数据库相关问题**
   - 验证 `database_path` 的权限
   - 检查磁盘空间
   - 定期备份数据库文件

## 总结

合理的 Memory 配置能够显著提升长对话的用户体验，同时控制系统资源消耗。建议：

1. **从推荐配置开始**，根据实际使用情况调整
2. **监控关键指标**，持续优化配置参数
3. **确保服务稳定**，特别是总结服务的可用性
4. **注意安全性**，保护敏感配置信息

通过合理配置和持续监控，Memory 功能将为您的应用提供出色的对话体验。
