<div align = "right">
<a href="memory-configuration-guide-zh.md">简体中文</a>
</div>

# LlamaNexus Memory Configuration Guide

This document provides detailed information about Memory feature configuration parameters in LlamaNexus, including usage methods, considerations, and best practices.

- [LlamaNexus Memory Configuration Guide](#llamanexus-memory-configuration-guide)
  - [Configuration Overview](#configuration-overview)
  - [Detailed Configuration](#detailed-configuration)
    - [1. enable](#1-enable)
    - [2. database\_path](#2-database_path)
    - [3. context\_window](#3-context_window)
    - [4. auto\_summarize](#4-auto_summarize)
    - [5. summary\_service\_base\_url](#5-summary_service_base_url)
    - [6. summary\_service\_api\_key](#6-summary_service_api_key)
    - [7. max\_stored\_messages](#7-max_stored_messages)
    - [8. summarize\_threshold](#8-summarize_threshold)
  - [Configuration Relationship Diagram](#configuration-relationship-diagram)
  - [Best Practices](#best-practices)
    - [1. Parameter Configuration Recommendations](#1-parameter-configuration-recommendations)
    - [2. Performance Optimization](#2-performance-optimization)
    - [3. Troubleshooting](#3-troubleshooting)
  - [Summary](#summary)

## Configuration Overview

The Memory feature helps maintain context coherence in long conversations through intelligent message management and automatic summarization mechanisms, while controlling memory usage.

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

## Detailed Configuration

### 1. enable

**Function**: Enable or disable the entire Memory functionality.

**Configuration**:

```toml
enable = true  # Enable Memory functionality
enable = false # Disable Memory functionality
```

**Usage**:

- `true`: Enable complete Memory functionality, including message storage, retrieval, and automatic summarization
- `false`: Disable Memory functionality, system will not store conversation history

**Considerations**:

- When disabled, all related memory configuration items will be inactive
- Recommended to enable in production environments for better conversation experience
- Disabling can save storage space and computational resources

---

### 2. database_path

**Function**: Specify the storage path for the SQLite database file.

**Configuration**:

```toml
database_path = "data/memory.db"
```

**Usage**:

- Supports both relative and absolute paths
- Relative paths are relative to the application's working directory
- File extension typically uses `.db`

**Considerations**:

- Ensure the specified directory exists and has write permissions
- Recommend regular database file backups
- Database file contains sensitive conversation history, pay attention to file security
- Path should not contain special characters or spaces

**Directory Structure Example**:

```bash
project/
├── data/
│   └── memory.db
├── config.toml
└── ...
```

---

### 3. context_window

**Function**: Define the maximum context window size for conversations (in token count).

**Configuration**:

```toml
context_window = 8192  # 8K tokens
```

**Usage**:

- Controls the maximum context length the system can "remember" when processing conversations
- When conversation exceeds this limit, automatic summarization mechanism is triggered
- Larger values can maintain longer context

**Recommended Configuration**:

- **Small applications**: 2048-4096 tokens
- **Medium applications**: 4096-8192 tokens
- **Large applications**: 8192-16384 tokens

**Considerations**:

- Must consider the actual context limitations of the target LLM
- Larger values consume more memory and computational resources
- Should be greater than the average token count corresponding to `max_stored_messages`
- Recommend monitoring and adjusting based on actual usage

---

### 4. auto_summarize

**Function**: Enable or disable automatic message summarization functionality.

**Configuration**:

```toml
auto_summarize = true  # Enable automatic summarization
auto_summarize = false # Disable automatic summarization
```

**Usage**:

- `true`: Automatically trigger summarization when message count reaches threshold
- `false`: Disable automatic summarization, may cause context overflow

**Considerations**:

- When enabled, requires configuration of `summary_service_base_url`
- Disabling may lead to context loss in long conversations
- Quality of summarization service directly affects conversation coherence
- Recommended to enable in production environments

---

### 5. summary_service_base_url

**Function**: Specify the base URL for external service used for message summarization.

**Configuration**:

```toml
summary_service_base_url = "http://localhost:10086/v1"
```

**Usage**:

- Provide complete HTTP/HTTPS URL
- Usually points to services compatible with OpenAI API format
- Supports both local deployment and cloud services

**URL Format Examples**:

```toml
# Local service
summary_service_base_url = "http://localhost:10086/v1"

# Remote service
summary_service_base_url = "https://api.openai.com/v1"

# Custom service
summary_service_base_url = "https://your-custom-llm-service.com/v1"
```

**Considerations**:

- Ensure service reachability and stability
- Verify service API compatibility
- Consider network latency impact on summarization speed
- Recommend using HTTPS in production environments

---

### 6. summary_service_api_key

**Function**: API key for accessing the summarization service.

**Configuration**:

```toml
summary_service_api_key = ""          # No API key required
summary_service_api_key = "sk-xxx..."  # OpenAI format key
```

**Usage**:

- Empty string indicates no authentication required
- Usually used for cloud services or APIs requiring authentication
- Supports various API key formats

**Considerations**:

- **Security**: Keys contain sensitive information, protect carefully
- **Environment Variables**: Recommend passing keys through environment variables
- **Access Control**: Ensure keys have appropriate permissions
- **Regular Rotation**: Regularly update API keys

**Security Best Practices**:

```bash
# Use environment variables
export SUMMARY_API_KEY="your-api-key"
```

---

### 7. max_stored_messages

**Function**: Set the message count threshold for triggering automatic summarization.

**Configuration**:

```toml
max_stored_messages = 20  # Trigger summarization at 20 messages
```

**Usage**:

- When stored message count reaches this value, automatically trigger summarization process
- Smaller values trigger frequent summarization, larger values may cause excessive context
- Needs to work in conjunction with `summarize_threshold`

**Recommended Configuration**:

- **Short conversation scenarios**: 10-15 messages
- **Medium conversation scenarios**: 15-25 messages
- **Long conversation scenarios**: 25-40 messages

**Considerations**:

- Must be greater than `summarize_threshold`
- Consider average message length
- Frequent summarization increases computational overhead
- Should adjust based on actual usage patterns

---

### 8. summarize_threshold

**Function**: Define the calculation base for the number of recent messages to retain after summarization.

**Configuration**:

```toml
summarize_threshold = 12  # Retain threshold/2 = 6 recent messages
```

**Usage**:

- Number of messages retained after summarization = `summarize_threshold / 2`
- Older messages are summarized into concise summaries
- Ensures important recent context is preserved

**Workflow Example**:

```txt
Initial state: 20 messages (reached max_stored_messages)
↓
Trigger summarization
↓
Retain recent 6 messages (summarize_threshold/2 = 12/2 = 6)
Summarize previous 14 messages into summary
↓
Final state: 1 summary + 6 original messages
```

**Considerations**:

- Must be less than `max_stored_messages`
- Recommend `max_stored_messages` be 1.5-2 times `summarize_threshold`
- Number of retained messages directly affects context coherence
- Number of summarized messages affects summary detail level

## Configuration Relationship Diagram

```txt
max_stored_messages (20) > summarize_threshold (12)
                              ↓
                        kept_messages = 12/2 = 6
                              ↓
                    summarized_messages = 20 - 6 = 14
```

## Best Practices

### 1. Parameter Configuration Recommendations

```toml
# Balanced configuration (recommended)
[memory]
enable = true
context_window = 8192
max_stored_messages = 20
summarize_threshold = 12
auto_summarize = true

# High-frequency conversation configuration
[memory]
enable = true
context_window = 4096
max_stored_messages = 15
summarize_threshold = 10
auto_summarize = true

# Long conversation configuration
[memory]
enable = true
context_window = 16384
max_stored_messages = 30
summarize_threshold = 18
auto_summarize = true
```

### 2. Performance Optimization

- **Monitoring Metrics**:
  - Average conversation length
  - Summarization trigger frequency
  - Memory usage
  - Response time

- **Tuning Strategies**:
  - Adjust thresholds based on actual conversation patterns
  - Monitor summarization service performance
  - Regularly clean up expired data
  - Optimize database queries

### 3. Troubleshooting

**Common Issues**:

1. **Summarization Service Unavailable**
   - Check `summary_service_base_url` reachability
   - Verify API key validity
   - Consider setting up backup summarization service

2. **High Memory Usage**
   - Reduce `context_window` value
   - Lower `max_stored_messages`
   - Increase summarization frequency

3. **Conversation Context Loss**
   - Increase `summarize_threshold` value
   - Check summarization service quality
   - Consider adjusting summarization prompts

4. **Database Related Issues**
   - Verify `database_path` permissions
   - Check disk space
   - Regularly backup database files

## Summary

Proper Memory configuration can significantly improve user experience in long conversations while controlling system resource consumption. Recommendations:

1. **Start with recommended configurations**, adjust based on actual usage
2. **Monitor key metrics**, continuously optimize configuration parameters
3. **Ensure service stability**, especially summarization service availability
4. **Pay attention to security**, protect sensitive configuration information

Through proper configuration and continuous monitoring, the Memory feature will provide excellent conversation experience for your application.
