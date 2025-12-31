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
      - [2.1 Simple File Path (Automatic Mode)](#21-simple-file-path-automatic-mode)
      - [2.2 Full SQLite URL (Advanced Mode)](#22-full-sqlite-url-advanced-mode)
      - [2.3 In-Memory Database (Development/Testing)](#23-in-memory-database-developmenttesting)
    - [3. context\_window](#3-context_window)
    - [4. auto\_summarize](#4-auto_summarize)
    - [5. summarization\_strategy](#5-summarization_strategy)
    - [6. summary\_service\_base\_url](#6-summary_service_base_url)
    - [7. summary\_service\_api\_key](#7-summary_service_api_key)
    - [8. max\_stored\_messages](#8-max_stored_messages)
    - [9. summarize\_threshold](#9-summarize_threshold)
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
database_path = "data/memory.db"                 # Simple file path (recommended)
# database_path = "sqlite:data/memory.db?mode=rwc&cache=shared"  # Full URL format
# database_path = "sqlite::memory:"              # In-memory database (dev/test)
context_window = 8192
auto_summarize = true
summarization_strategy = "Incremental"
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

**Function**: Specify the storage path or connection URL for the SQLite database file.

**Configuration Formats**:

```toml
# Method 1: Simple file path (recommended)
database_path = "data/memory.db"

# Method 2: Full SQLite URL
database_path = "sqlite:data/memory.db?mode=rwc"

# Method 3: In-memory database (temporary use)
database_path = "sqlite::memory:"
```

**Supported Format Details**:

#### 2.1 Simple File Path (Automatic Mode)

```toml
# Relative paths
database_path = "data/memory.db"
database_path = "storage/conversations.db"

# Absolute paths
database_path = "/var/lib/llama-nexus/memory.db"
database_path = "/home/user/apps/memory.db"

# Subdirectory paths (auto-create directories)
database_path = "custom/path/database.db"
```

**Features**:

- System automatically adds `sqlite:` protocol and `?mode=rwc` parameters
- Automatically creates non-existent parent directories
- Simplest configuration method, recommended for daily use

#### 2.2 Full SQLite URL (Advanced Mode)

```toml
# Basic URL format
database_path = "sqlite:data/memory.db?mode=rwc"

# With cache parameters
database_path = "sqlite:data/memory.db?mode=rwc&cache=shared"

# Absolute path URL
database_path = "sqlite:///var/lib/app/memory.db?mode=rwc"

# With timeout parameters
database_path = "sqlite:data/memory.db?mode=rwc&timeout=30"
```

**URL Parameter Descriptions**:

- `mode=rwc`: r(read) + w(write) + c(create), required parameter
- `cache=shared`: Enable shared cache for improved multi-connection performance
- `timeout=30`: Set connection timeout (seconds)

**Features**:

- Full control over SQLite connection parameters
- Suitable for advanced users and special requirements
- Allows fine-tuning database performance

#### 2.3 In-Memory Database (Development/Testing)

```toml
database_path = "sqlite::memory:"
```

**Features**:

- Data stored in memory with extreme performance
- Data lost after service restart
- Suitable for development, testing, and temporary use
- No file system permissions required

**Use Case Comparison**:

| Configuration Method | Use Case | Advantages | Disadvantages |
|---------------------|----------|------------|---------------|
| Simple Path | Daily production use | Simple config, auto-managed | Limited parameter control |
| Full URL | Advanced configuration needs | Full control, performance tuning | Complex configuration |
| In-Memory Database | Development/testing | Highest performance, no file dependencies | Data not persistent |

**Considerations**:

- **Permission Management**: Ensure specified directory exists and has write permissions
- **Security**: Database file contains sensitive conversation history, pay attention to file security
- **Backup Strategy**: Recommend regular database file backups
- **Path Format**: Avoid special characters in paths, recommend using English and numbers
- **Auto-Creation**: System automatically creates non-existent parent directories and database files

**Directory Structure Example**:

```bash
project/
├── data/                    # Default data directory
│   └── memory.db           # Main database file
├── custom/                 # Custom directory
│   └── path/
│       └── database.db     # Custom path database
├── config.toml             # Configuration file
└── logs/                   # Log directory
```

**Configuration Migration Guide**:

If you're already using the old simple path configuration, no changes are needed. The new version is fully backward compatible:

```toml
# Old configuration (still valid)
database_path = "data/memory.db"

# Equivalent new URL format
database_path = "sqlite:data/memory.db?mode=rwc"
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

### 5. summarization_strategy

**Function**: Choose summarization strategy to control how summaries are generated and their quality.

**Configuration**:

```toml
summarization_strategy = "Incremental"  # Incremental summarization strategy (default)
summarization_strategy = "FullHistory"  # Full history summarization strategy
```

**Strategy Descriptions**:

**Incremental Summarization (Incremental)**:

- **How it works**: Generate updated summary based on existing summary + new messages
- **Advantages**: High efficiency, low computational overhead, fast response time
- **Disadvantages**: May lose some context information over time
- **Use cases**: High-frequency conversations, resource-constrained environments, simple dialogues

**Full History Summarization (FullHistory)**:

- **How it works**: Regenerate summary based on all relevant historical messages
- **Advantages**: Complete context, high summary quality, preserves information integrity
- **Disadvantages**: High computational overhead, longer response time
- **Use cases**: Complex tasks, long-term conversations, high-quality requirements

**Configuration Recommendations**:

```toml
# High-frequency scenario configuration
[memory]
summarization_strategy = "Incremental"
max_stored_messages = 15
summarize_threshold = 10
context_window = 4096

# High-quality scenario configuration
[memory]
summarization_strategy = "FullHistory"
max_stored_messages = 25
summarize_threshold = 15
context_window = 16384
```

**Performance Comparison**:

| Strategy Type | Response Time | Computational Cost | Summary Quality | Context Preservation |
|---------------|---------------|---------------------|-----------------|---------------------|
| Incremental | Fast | Low | Medium | May decrease |
| Full History | Slow | High | High | Complete |

**Considerations**:

- Default is `Incremental` strategy for backward compatibility
- `FullHistory` strategy significantly increases computation time and resource consumption
- Choose appropriate strategy based on specific application scenarios
- Configuration can be adjusted dynamically at runtime

---

### 6. summary_service_base_url

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

### 7. summary_service_api_key

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

### 8. max_stored_messages

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

### 9. summarize_threshold

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
database_path = "data/memory.db"                # Simple path, auto-managed
context_window = 8192
max_stored_messages = 20
summarize_threshold = 12
auto_summarize = true
summarization_strategy = "Incremental"

# High-frequency conversation configuration
[memory]
enable = true
database_path = "sqlite:data/memory.db?mode=rwc&cache=shared"  # Enable cache optimization
context_window = 4096
max_stored_messages = 15
summarize_threshold = 10
auto_summarize = true
summarization_strategy = "Incremental"

# High-quality long conversation configuration
[memory]
enable = true
database_path = "storage/conversations.db"      # Dedicated storage directory
context_window = 16384
max_stored_messages = 25
summarize_threshold = 15
auto_summarize = true
summarization_strategy = "FullHistory"

# Development/testing configuration
[memory]
enable = true
database_path = "sqlite::memory:"               # In-memory database, lost on restart
context_window = 4096
max_stored_messages = 10
summarize_threshold = 6
auto_summarize = true
summarization_strategy = "Incremental"
```

**Summarization Strategy Selection Guide**:

- **Choose Incremental Summarization when**:
  - High-frequency conversation interactions
  - Strict response time requirements
  - Resource-constrained deployment environments
  - Relatively simple conversation content

- **Choose Full History Summarization when**:
  - Complex multi-turn task conversations
  - Need to maintain complete context
  - Extremely high summary quality requirements
  - Important decision-support conversations

### 2. Performance Optimization

- **Monitoring Metrics**:
  - Average conversation length
  - Summarization trigger frequency
  - Memory usage
  - Response time
  - **Summary generation time**:
    - Incremental summarization: usually < 2 seconds
    - Full history summarization: may be > 5 seconds

- **Tuning Strategies**:
  - Adjust thresholds based on actual conversation patterns
  - Monitor summarization service performance
  - Regularly clean up expired data
  - Optimize database queries
  - **Summarization strategy tuning**:
    - Choose appropriate summarization strategy based on load conditions
    - Use incremental summarization under high load
    - Use full history summarization when high quality is required
    - Monitor summary quality and adjust accordingly

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
   - Verify `database_path` permissions and format
   - Check disk space
   - Confirm directories are auto-created successfully
   - Test SQLite URL format correctness
   - Verify SQLite connection parameters
   - Regularly backup database files

5. **database_path Configuration Issues**
   - **Simple path cannot be created**: Check parent directory permissions, ensure application has write access
   - **URL format connection failure**: Verify URL syntax, ensure `mode=rwc` parameter is included
   - **In-memory database data loss**: Confirm using `sqlite::memory:` is expected behavior
   - **Path contains special characters**: Use URL encoding or avoid special characters
   - **Relative path issues**: Confirm working directory, recommend using absolute paths

6. **Summarization Strategy Related Issues**
   - **Full history summarization responds slowly**: Consider switching to incremental summarization strategy
   - **Incremental summarization quality degradation**: Periodically use full history summarization to regenerate baseline summaries
   - **Incoherent summary content**: Check summarization service configuration and prompt settings
   - **Historical information loss**: Consider using full history summarization strategy

## Summary

Proper Memory configuration can significantly improve user experience in long conversations while controlling system resource consumption. Recommendations:

1. **Start with recommended configurations**, adjust based on actual usage
2. **Choose appropriate database configuration**:
   - Use simple file path configuration for production environments
   - Use URL format configuration for high-performance requirements
   - Use in-memory database for development and testing
3. **Monitor key metrics**, continuously optimize configuration parameters
4. **Ensure service stability**, especially summarization service availability
5. **Pay attention to security**, protect sensitive configuration information and database files

Through proper configuration and continuous monitoring, the Memory feature will provide excellent conversation experience for your application. The new database_path configuration options provide greater flexibility for different use cases.
4. **Pay attention to security**, protect sensitive configuration information

Through proper configuration and continuous monitoring, the Memory feature will provide excellent conversation experience for your application.
