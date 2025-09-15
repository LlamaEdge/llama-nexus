# Llama-Nexus

Llama-Nexus is a gateway service for managing and orchestrating LlamaEdge API servers. It provides a unified interface to various AI services including chat completions, audio processing, image generation, and text-to-speech capabilities. Compatible with OpenAI API, Llama-Nexus allows you to use familiar API formats while working with open-source models. With Llama-Nexus, you can easily register and manage multiple API servers, handle requests, and monitor the health of your AI services.

- [Llama-Nexus](#llama-nexus)
  - [Installation](#installation)
  - [Usage](#usage)
  - [Command Line Usage](#command-line-usage)
  - [Development](#development)
    - [Prerequisites](#prerequisites)
    - [Building from Source](#building-from-source)
    - [CI/CD Notes](#cicd-notes)
    - [Troubleshooting](#troubleshooting)

## Installation

- Download Llama-Nexus binary

  The Llama-Nexus binaries can be found at the [release page](https://github.com/llamaedge/llamaedge-nexus/releases). To download the binary, you can use the following command:

  ```bash
  # Download the binary for Linux x86_64
  curl -L https://github.com/LlamaEdge/llama-nexus/releases/latest/download/llama-nexus-unknown-linux-gnu-aarch64.tar.gz -o llama-nexus.tar.gz

  # Download the binary for Linux ARM64
  curl -L https://github.com/LlamaEdge/llama-nexus/releases/latest/download/llama-nexus-unknown-linux-gnu-x86_64.tar.gz -o llama-nexus.tar.gz

  # Download the binary for macOS x86_64
  curl -L https://github.com/LlamaEdge/llama-nexus/releases/latest/download/llama-nexus-apple-darwin-x86_64.tar.gz -o llama-nexus.tar.gz

  # Download the binary for macOS ARM64
  curl -L https://github.com/LlamaEdge/llama-nexus/releases/latest/download/llama-nexus-apple-darwin-aarch64.tar.gz -o llama-nexus.tar.gz

  # Extract the binary
  tar -xzf llama-nexus.tar.gz
  ```

  After decompressing the file, you will see the following files in the current directory.

  ```bash
  llama-nexus
  config.toml
  SHA256SUMS
  ```

- Download LlamaEdge API Servers

  LlamaEdge provides four types of API servers:

  - `llama-api-server` provides chat and embedding APIs. [Release Page](https://github.com/LlamaEdge/LlamaEdge/releases)
  - `whisper-api-server` provides audio transcription and translation APIs. [Release Page](https://github.com/LlamaEdge/whisper-api-server/releases)
  - `sd-api-server` provides image generation and editing APIs. [Release Page](https://github.com/LlamaEdge/sd-api-server/releases)
  - `tts-api-server` provides text-to-speech APIs. [Release Page](https://github.com/LlamaEdge/tts-api-server/releases)

  To download the `llama-api-server`, for example, use the following command:

  ```bash
  curl -L https://github.com/LlamaEdge/LlamaEdge/releases/latest/download/llama-api-server.wasm -o llama-api-server.wasm
  ```

- Install WasmEdge Runtime

  ```bash
  # To run models on CPU
  curl -sSf https://raw.githubusercontent.com/WasmEdge/WasmEdge/master/utils/install_v2.sh | bash -s -- -v 0.14.1

  # To run models on NVIDIA GPU with CUDA 12
  curl -sSf https://raw.githubusercontent.com/WasmEdge/WasmEdge/master/utils/install_v2.sh | bash -s -- -v 0.14.1 --ggmlbn=12

  # To run models on NVIDIA GPU with CUDA 11
  curl -sSf https://raw.githubusercontent.com/WasmEdge/WasmEdge/master/utils/install_v2.sh | bash -s -- -v 0.14.1 --ggmlbn=11
  ```

- Start Llama-Nexus

  Run the following command to start Llama-Nexus:

  ```bash
  # Start Llama-Nexus with the default config file at default port 3389
  llama-nexus --config config.toml
  ```

  For the details about the CLI options, please refer to the [Command Line Usage](#command-line-usage) section.

- Register LlamaEdge API Servers to Llama-Nexus

  Run the following commands to start LlamaEdge API Servers first:

  ```bash
  # Download a gguf model file, for example, Llama-3.2-3B-Instruct-Q5_K_M.gguf
  curl -LO https://huggingface.co/second-state/Llama-3.2-3B-Instruct-GGUF/resolve/main/Llama-3.2-3B-Instruct-Q5_K_M.gguf

  # Start LlamaEdge API Servers
  wasmedge --dir .:. --nn-preload default:GGML:AUTO:Llama-3.2-3B-Instruct-Q5_K_M.gguf \
    llama-api-server.wasm \
    --prompt-template llama-3-chat \
    --ctx-size 128000 \
    --model-name Llama-3.2-3b \
    --port 10010
  ```

  Then, register the LlamaEdge API Servers to Llama-Nexus:

- **Option 1: Manual API Registration (Recommended)**

  Alternatively, you can manually register services via REST API after starting Llama-Nexus:

  ```bash
  curl --location 'http://localhost:3389/admin/servers/register' \
  --header 'Content-Type: application/json' \
  --data '{
      "url": "http://localhost:10010/v1",
      "kind": "chat",
      "api_key": "Bearer <your-api-key>"
  }'
  ```

  > The `kind` can be `chat`, `embeddings`, `image`, `transcribe`, `translate`, or `tts`.
  > The `api_key` is optional. If the `api_key` is provided, it will be used to authenticate the request to the downstream server.

  If register successfully, you will see a similar response like:

  ```bash
  {
      "id": "chat-server-36537062-9bea-4234-bc59-3166c43cf3f1",
      "kind": "chat",
      "url": "http://localhost:10010/v1"
  }
  ```

- **Option 2: Configuration-based Registration (Recommended)**

  You can pre-configure AI services in your `config.toml` file. These services will be automatically registered when Llama-Nexus starts:

  ```toml
  # Uncomment and configure the services you need
  [chat]
  url = "http://localhost:10010/v1"  # Your chat service URL
  api_key = ""                      # Leave empty to use DEFAULT_CHAT_SERVICE_API_KEY env var

  [embedding]
  url = "http://localhost:10011/v1"  # Your embedding service URL
  api_key = ""                      # Leave empty to use DEFAULT_EMBEDDING_SERVICE_API_KEY env var
  ```

  **Environment Variable Support:**

  If you prefer to keep API keys in environment variables:

  ```bash
  export DEFAULT_CHAT_SERVICE_API_KEY="your-chat-api-key"
  export DEFAULT_EMBEDDING_SERVICE_API_KEY="your-embedding-api-key"
  ```

  Then start Llama-Nexus:

  ```bash
  llama-nexus --config config.toml
  ```

  The configured services will be automatically registered and available immediately.

## Usage

If you finish registering a chat server into Llama-Nexus, you can send a chat-completion request to the port Llama-Nexus is listening on. For example, you can use the following command to send a chat-completion request to the port `3389`:

```bash
curl --location 'http://localhost:3389/v1/chat/completions' \
--header 'Content-Type: application/json' \
--data '{
    "model": "Llama-3.2-3b",
    "messages": [
        {
            "role": "system",
            "content": "You are an AI assistant. Answer questions as concisely and accurately as possible."
        },
        {
            "role": "user",
            "content": "What is the capital of France?"
        },
        {
            "content": "Paris",
            "role": "assistant"
        },
        {
            "role": "user",
            "content": "How many planets are in the solar system?"
        }
    ],
    "stream": false
}'
```

## Command Line Usage

Llama-Nexus provides various command line options to configure the service behavior. You can specify the config file path, enable RAG functionality, set up health checks, configure the Web UI, and manage logging. Here are the available command line options by running `llama-nexus --help`:

```bash
LlamaEdge Nexus - A gateway service for LLM backends

Usage: llama-nexus [OPTIONS]

Options:
      --config <CONFIG>
          Path to the config file [default: config.toml]
      --check-health
          Enable health check for downstream servers
      --check-health-interval <CHECK_HEALTH_INTERVAL>
          Health check interval for downstream servers in seconds [default: 60]
      --web-ui <WEB_UI>
          Root path for the Web UI files [default: chatbot-ui]
      --log-destination <LOG_DESTINATION>
          Log destination: "stdout", "file", or "both" [default: stdout]
      --log-file <LOG_FILE>
          Log file path (required when log_destination is "file" or "both")
  -h, --help
          Print help
  -V, --version
          Print version
```

## Development

This section provides guidance for developers who want to contribute to Llama-Nexus or build from source.

### Prerequisites

- [Rust](https://rustup.rs/) (latest stable version)

> **Note:** SQLite is automatically bundled with the project via `libsqlite3-sys` crate. No separate SQLite installation is required.

### Building from Source

1. **Clone the repository:**

   ```bash
   git clone https://github.com/LlamaEdge/llama-nexus.git
   cd llama-nexus
   ```

2. **Build the project:**

   ```bash
   # For development builds
   SQLX_OFFLINE=true cargo build

   # For release builds
   SQLX_OFFLINE=true cargo build --release
   ```

   > **Important:** Always use `SQLX_OFFLINE=true` during compilation to avoid requiring a database connection at build time.

3. **Run the application:**

   ```bash
   # Development mode
   cargo run

   # Release mode
   cargo run --release

   # Or run the binary directly
   ./target/release/llama-nexus
   ```

### CI/CD Notes

When setting up continuous integration:

- Use `SQLX_OFFLINE=true` for all build and test commands
- The `.sqlx/` directory contains query metadata and should be committed
- Database files (`data/memory.db`) should be excluded from version control
- No actual database instance is required for compilation or testing

### Troubleshooting

**Build Issues:**

- Ensure `SQLX_OFFLINE=true` is set during compilation
- Run `cargo sqlx prepare` after modifying SQL queries
- Check that `.sqlx/` directory is present and up-to-date

**Runtime Issues:**

- Ensure the `data/` directory exists (created automatically)
- Check database permissions if running in restricted environments
- Verify `memory.enable` setting in `config.toml` matches your requirements
