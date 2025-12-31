# Llama-nexus RAG Search: Configuration and Usage Guide

In RAG (Retrieval-Augmented Generation) scenarios, vector search and keyword search are two common search methods. llama-nexus supports both search methods through [Qdrant](https://qdrant.tech/) and [kw-search-server](https://github.com/LlamaEdge/kw-search-server). Additionally, llama-nexus also supports keyword search through [Elasticsearch](https://www.elastic.co/).

In this guide, we will use Qdrant and kw-search-server as examples to demonstrate:

- How to start llama-nexus in RAG mode along with related servers
- How to create embeddings and indexes for documents
- How to perform vector search and keyword search

## Table of Contents

- [Llama-nexus RAG Search: Configuration and Usage Guide](#llama-nexus-rag-search-configuration-and-usage-guide)
  - [Table of Contents](#table-of-contents)
  - [Starting llama-nexus and Related Servers](#starting-llama-nexus-and-related-servers)
    - [Configuring llama-nexus Startup Parameters](#configuring-llama-nexus-startup-parameters)
    - [Starting llama-nexus](#starting-llama-nexus)
  - [Creating Embeddings and Indices (Optional)](#creating-embeddings-and-indices-optional)
  - [Executing Search](#executing-search)

## Starting llama-nexus and Related Servers

First, download the `llama-nexus` binary:

```bash
export NEXUS_VERSION=0.3.0

# macOS on Apple Silicon
curl -LO https://github.com/LlamaEdge/llama-nexus/releases/download/{$NEXUS_VERSION}/llama-nexus-apple-darwin-aarch64.tar.gz
tar -xvzf llama-nexus-apple-darwin-aarch64.tar.gz llama-nexus config.toml

# macOS on Intel
curl -LO https://github.com/LlamaEdge/llama-nexus/releases/download/{$NEXUS_VERSION}/llama-nexus-apple-darwin-x86_64.tar.gz
tar -xvzf llama-nexus-apple-darwin-x86_64.tar.gz llama-nexus config.toml

# Linux on x86_64
curl -LO https://github.com/LlamaEdge/llama-nexus/releases/download/{$NEXUS_VERSION}/llama-nexus-unknown-linux-gnu-x86_64.tar.gz
tar -xvzf llama-nexus-unknown-linux-gnu-x86_64.tar.gz llama-nexus config.toml

# Linux on aarch64
curl -LO https://github.com/LlamaEdge/llama-nexus/releases/download/{$NEXUS_VERSION}/llama-nexus-unknown-linux-gnu-aarch64.tar.gz
tar -xvzf llama-nexus-unknown-linux-gnu-aarch64.tar.gz llama-nexus config.toml
```

The extracted files include the `llama-nexus` binary and `config.toml` configuration file. Next, we'll configure the `config.toml` file.

### Configuring llama-nexus Startup Parameters

llama-nexus manages various configuration items through the `config.toml` file. The `[rag]` configuration section defines RAG-related settings. To enable RAG mode, simply set `enable` to `true`.

```toml
(Other configuration items)

[rag]
enable         = true
policy         = "last-user-message"
context_window = 1

(Other configuration items)
```

Similar to enabling RAG mode, you can enable two MCP servers by configuring the `[mcp.server.vector_search]` and `[mcp.server.keyword_search]` sections in `config.toml`:

```toml
(Other configuration items)

[mcp.server.vector_search]
name      = "gaia-qdrant"
transport = "stream-http"
url       = "http://127.0.0.1:8003/mcp"
enable    = true

(Other configuration items)


[mcp.server.keyword_search]
name      = "gaia-keyword-search"
transport = "stream-http"
url       = "http://127.0.0.1:8005/mcp"
enable    = true

(Other configuration items)
```

### Starting llama-nexus

After configuring the startup parameters, follow these steps to start llama-nexus:

- Install WasmEdge Runtime

  - CPU Only

    ```bash
    # Version of WasmEdge Runtime
    export wasmedge_version="0.14.1"

    # Version of ggml plugin
    export ggml_plugin="b5593"

    # For CPU
    curl -sSf https://raw.githubusercontent.com/WasmEdge/WasmEdge/master/utils/install_v2.sh | bash -s -- -v $wasmedge_version --ggmlbn=$ggml_plugin
    ```

  - GPU

    ```bash
    # Version of WasmEdge Runtime
    export wasmedge_version="0.14.1"

    # Version of ggml plugin
    export ggml_plugin="b5593"

    # CUDA version: 11 or 12
    export ggmlcuda=12

    curl -sSf https://raw.githubusercontent.com/WasmEdge/WasmEdge/master/utils/install_v2.sh | bash -s -- -v $wasmedge_version --ggmlbn=$ggml_plugin --ggmlcuda=$ggmlcuda
    ```

- Start llama-api-server

  First, download `llama-api-server.wasm` and models:

  ```bash
  # Download llama-api-server.wasm
  export API_SERVER_VERSION=0.22.0
  curl -LO https://github.com/LlamaEdge/LlamaEdge/releases/download/{$API_SERVER_VERSION}/llama-api-server.wasm

  # Download chat model
  curl -LO https://huggingface.co/second-state/Qwen3-4B-GGUF/resolve/main/Qwen3-4B-Q5_K_M.gguf

  # Download embedding model
  curl -LO https://huggingface.co/second-state/nomic-embed-text-v1.5-GGUF/resolve/main/nomic-embed-text-v1.5-f16.gguf
  ```

  Then, start the chat and embedding servers:

  ```bash
  # start chat server
  wasmedge --dir .:. --nn-preload default:GGML:AUTO:Qwen3-4B-Q5_K_M.gguf \
  llama-api-server.wasm \
  --model-name Qwen3-4B \
  --prompt-template qwen3-no-think \
  --ctx-size 8192 --port 9068

  # start embedding server
  wasmedge --dir .:. --nn-preload default:GGML:AUTO:nomic-embed-text-v1.5-f16.gguf \
  llama-api-server.wasm \
  --prompt-template embedding \
  --ctx-size 768 \
  --model-name nomic-embed-text-v1.5 --port 9069
  ```

  If started successfully, the chat server and embedding server will run on ports `9068` and `9069` respectively.

- Start MCP servers

  First, download `gaia-qdrant-mcp-server-sse` and `gaia-kwsearch-mcp-server-sse` binaries:

  ```bash
  export MCP_SERVER_VERSION=0.4.0

  # macOS on Apple Silicon
  curl -LO https://github.com/decentralized-mcp/gaia-mcp-servers/releases/download/{$MCP_SERVER_VERSION}/gaia-mcp-servers-apple-darwin-aarch64.tar.gz
  tar -xvzf gaia-mcp-servers-apple-darwin-aarch64.tar.gz gaia-qdrant-mcp-server gaia-kwsearch-mcp-server

  # macOS on Intel
  curl -LO https://github.com/decentralized-mcp/gaia-mcp-servers/releases/download/{$MCP_SERVER_VERSION}/gaia-mcp-servers-apple-darwin-x86_64.tar.gz
  tar -xvzf gaia-mcp-servers-apple-darwin-x86_64.tar.gz gaia-qdrant-mcp-server gaia-kwsearch-mcp-server

  # Linux on x86_64
  curl -LO https://github.com/decentralized-mcp/gaia-mcp-servers/releases/download/{$MCP_SERVER_VERSION}/gaia-mcp-servers-linux-unknown-gnu-x86_64.tar.gz
  tar -xvzf gaia-mcp-servers-linux-unknown-gnu-x86_64.tar.gz gaia-qdrant-mcp-server gaia-kwsearch-mcp-server

  # Linux on aarch64
  curl -LO https://github.com/decentralized-mcp/gaia-mcp-servers/releases/download/{$MCP_SERVER_VERSION}/gaia-mcp-servers-linux-unknown-gnu-aarch64.tar.gz
  tar -xvzf gaia-mcp-servers-linux-unknown-gnu-aarch64.tar.gz gaia-qdrant-mcp-server gaia-kwsearch-mcp-server
  ```

  Then, start the MCP servers:

  ```bash
  # Start gaia-qdrant mcp server
  ./gaia-qdrant-mcp-server --base-url http://127.0.0.1:6333 --transport stream-http --collection <your-collection-name>

  # Start gaia-keyword-search mcp server
  ./gaia-kwsearch-mcp-server --index <your-index-name> --transport stream-http
  ```

  If started successfully, the `gaia-qdrant-mcp-server` and `gaia-kwsearch-mcp-server` MCP servers will run on ports `8003` and `8005` respectively.

  Note: The `gaia-qdrant-mcp-server` and `gaia-kwsearch-mcp-server` MCP servers will access Qdrant and kw-search-server respectively when performing searches. Ensure that Qdrant and kw-search-server are properly started and running on their default ports `6333` and `12306`:

  <details><summary>Expand to view the steps to deploy Qdrant and kw-search-server</summary>

  - Deploy and Start Qdrant Locally

    First, download the latest Qdrant image from Dockerhub:

    ```bash
    docker pull qdrant/qdrant
    ```

    Then, run the service:

    ```bash
    docker run -p 6333:6333 -p 6334:6334 \
        -v "$(pwd)/qdrant_storage:/qdrant/storage:z" \
        qdrant/qdrant
    ```

  - Deploy and Start kw-search-server Locally

    First, download the `kw-search-server` binary:

    ```bash
    export KW_SERVER_VERSION=0.2.0

    # macOS on Apple Silicon
    curl -LO https://github.com/LlamaEdge/kw-search-server/releases/download/{$KW_SERVER_VERSION}/kw-search-server-aarch64-apple-darwin.tar.gz
    tar -xvzf kw-search-server-aarch64-apple-darwin.tar.gz kw-search-server

    # macOS on Intel
    curl -LO https://github.com/LlamaEdge/kw-search-server/releases/download/{$KW_SERVER_VERSION}/kw-search-server-x86_64-apple-darwin.tar.gz
    tar -xvzf kw-search-server-x86_64-apple-darwin.tar.gz kw-search-server

    # Linux on x86_64
    curl -LO https://github.com/LlamaEdge/kw-search-server/releases/download/{$KW_SERVER_VERSION}/kw-search-server-x86_64-unknown-linux-gnu.tar.gz
    tar -xvzf kw-search-server-x86_64-unknown-linux-gnu.tar.gz kw-search-server

    # Linux on aarch64
    curl -LO https://github.com/LlamaEdge/kw-search-server/releases/download/{$KW_SERVER_VERSION}/kw-search-server-aarch64-unknown-linux-gnu.tar.gz
    tar -xvzf kw-search-server-aarch64-unknown-linux-gnu.tar.gz kw-search-server
    ```

    Then, run the service:

    ```bash
    # Run
    ./kw-search-server
    ```

    If started successfully, kw-search-server will run on port `12306` by default.

  </details>

- Start llama-nexus

  ```bash
  ./llama-nexus
  ```

  If started successfully, llama-nexus will run on port `3389` by default.

- Register chat and embedding servers

  ```bash
  # Register chat server
  curl --location 'http://localhost:3389/admin/servers/register' \
  --header 'Content-Type: application/json' \
  --data '{
      "url": "http://localhost:9068",
      "kind": "chat"
  }'

  # Register embedding server
  curl --location 'http://localhost:3389/admin/servers/register' \
  --header 'Content-Type: application/json' \
  --data '{
      "url": "http://localhost:9069",
      "kind": "embeddings"
  }'
  ```

At this point, llama-nexus and related servers are successfully started. The topology diagram is as follows:

<div align="center">
  <img src="./images/llama-nexus-topology.jpg" width="80%" />
</div>

Next, we'll create embeddings and indexes for documents.

## Creating Embeddings and Indices (Optional)

Before performing vector search and keyword search, you need to create embeddings and indexes for your documents. Embeddings will be stored in Qdrant, while indexes will be used by kw-search-server. llama-nexus provides the `/v1/create/rag` endpoint for creating embeddings and indexes for documents.

> [!NOTE]
> The `/v1/create/rag` endpoint currently only supports persisting embeddings in Qdrant and creating indexes for kw-search-server. Support for Elasticsearch will be added in the near future.

<details><summary>Expand to view the details of creating embeddings and indices</summary>

The CURL command below sends a request to `llama-nexus`, which performs the following tasks in sequence:

- Segments [paris.txt]() into chunks, with each chunk containing 150 words
- Creates an index named `paris-index-01` in `kw-search-server` running on `http://localhost:12306`
- Computes embeddings for the chunks and persists them to the collection named `paris-01` in Qdrant server running on `http://localhost:6333`

```bash
# Create embeddings and index
curl --location 'http://localhost:3389/v1/create/rag' \
--header 'Content-Type: multipart/form-data' \
--form 'file=@"paris.txt"' \
--form 'chunk_capacity="150"' \
--form 'vdb_server_url="http://localhost:6333"' \
--form 'vdb_collection_name="paris-01"' \
--form 'kw_search_url="http://localhost:12306"' \
--form 'kw_search_index_name="paris-index-01"'
```

If the request is processed successfully, a response similar to the following will be returned:

```bash
{
    "index": {
        "index_name": "paris-index-01",
        "results": [
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            },
            {
                "status": "indexed"
            }
        ]
    },
    "vdb": "Collection `paris-01` created successfully."
}
```

</details>

At this point, we have created embeddings and indexes for the documents. Next, we can perform vector search and keyword search.

## Executing Search

When llama-nexus is running in RAG mode with `gaia-qdrant-mcp-server` and `gaia-kwsearch-mcp-server` MCP servers enabled, sending a chat completion request will trigger vector search and keyword search.

The CURL command below sends a chat completion request to `llama-nexus`. Note: If your llama-nexus is running on a port other than `3389`, please update the port number in the request.

```bash
curl --location 'http://localhost:3389/v1/chat/completions' \
--header 'Content-Type: application/json' \
--data '{
    "model": "Qwen3-4B",
    "messages": [
        {
            "role": "system",
            "content": "You are a helpful assistant. Answer questions as concisely as possible."
        },
        {
            "role": "user",
            "content": "What is the location of Paris, France along the Seine river?"
        }
    ],
    "limit": 5,
    "score_threshold": 0.5,
}'
```

After processing, llama-nexus will return a response similar to:

```bash
{
    "id": "chatcmpl-49caad8c-24ee-4425-bae5-cbe7fc5dc8a8",
    "object": "chat.completion",
    "created": 1747733116,
    "model": "Qwen3-4B",
    "choices": [
        {
            "index": 0,
            "message": {
                "content": "Paris, France is located along the Seine River, approximately 233 miles (375 km) upstream from the river's mouth.",
                "role": "assistant"
            },
            "finish_reason": "stop",
            "logprobs": null
        }
    ],
    "usage": {
        "prompt_tokens": 285,
        "completion_tokens": 34,
        "total_tokens": 319
    }
}
```

At this point, we have completed the execution of vector search and keyword search. The results of vector search and keyword search can be viewed in the llama-nexus logs. Below is an example of the execution results from the llama-nexus logs:

<details><summary>Expand to view logs</summary>

```bash
(more logs)

2025-05-20T09:25:14.545208Z  INFO ThreadId(06) src/rag.rs:49: Received a new chat request - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.545232Z  INFO ThreadId(06) src/rag.rs:56: URL to the kw-search mcp-server: http://localhost:12306 - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.581827Z  INFO ThreadId(08) src/rag.rs:128: Got 5 kw-search hits - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.581838Z  INFO ThreadId(08) src/rag.rs:564: Use the VectorDB settings from the request - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.581842Z  INFO ThreadId(08) src/rag.rs:580: qdrant url: http://localhost:6333, collection name: paris-03, limit: 5, score threshold: 0.5 - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.581846Z  INFO ThreadId(08) src/rag.rs:690: Computing embeddings for user query - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.581849Z  INFO ThreadId(08) src/rag.rs:703: Context window: 1 - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.581851Z  INFO ThreadId(08) src/rag.rs:743: Found the latest 1 user message(s) - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.581857Z  INFO ThreadId(08) src/rag.rs:761: Query text for the context retrieval: What is the location of Paris, France along the Seine river? - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.581861Z  INFO ThreadId(08) src/handlers.rs:695: Received a new embeddings request - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.581865Z  INFO ThreadId(08) src/handlers.rs:723: Forward the embeddings request to http://localhost:9069/v1/embeddings - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.628118Z  INFO ThreadId(06) src/handlers.rs:794: Embeddings request completed successfully - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.628174Z  INFO ThreadId(06) src/rag.rs:876: Retrieve context from http://localhost:6333/collections/paris-03, max number of result to return: 5, score threshold: 0.5 - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.673742Z  INFO ThreadId(04) src/rag.rs:948: Got 5 points from the gaia-qdrant-mcp-server in 0.00433625 seconds - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.673758Z  INFO ThreadId(04) src/rag.rs:964: Try to remove duplicated points - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.674140Z  INFO ThreadId(04) src/rag.rs:855: Retrieved 5 point(s) from the collection `paris-03` - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.674169Z  INFO ThreadId(04) src/rag.rs:335: em_hits_map: {
    16545325271760060286: RagScoredPoint {
        source: "Paris, city and capital of France, situated in the north-central part of the country.",
        score: 0.8027477,
    },
    13314534164983550839: RagScoredPoint {
        source: "Paris occupies a central position in the rich agricultural region known as the Paris Basin, and it constitutes one of eight départements of the Île-de",
        score: 0.7360471,
    },
    14397306707317103710: RagScoredPoint {
        source: "People were living on the site of the present-day city, located along the Seine River some 233 miles (375 km) upstream from the river's mouth on the",
        score: 0.7763241,
    },
    8064019955072080751: RagScoredPoint {
        source: "Paris's site at a crossroads of both water and land routes significant not only to France but also to Europe has had a continuing influence on its",
        score: 0.7246754,
    },
    656120371065341945: RagScoredPoint {
        source: "For centuries Paris has been one of the world's most important and attractive cities.",
        score: 0.6880192,
    },
} - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.674211Z  INFO ThreadId(04) src/rag.rs:344: em_scores: {
    14397306707317103710: 0.769685823487625,
    8064019955072080751: 0.319503872185202,
    656120371065341945: 0.0,
    13314534164983550839: 0.41862222551501976,
    16545325271760060286: 1.0,
} - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.674232Z  INFO ThreadId(04) src/rag.rs:355: kw_hits_map: {
    17408515414919045690: KwSearchHit {
        title: "Unknown",
        content: "-France administrative region. It is by far the country's most important centre of commerce and culture.",
        score: 5.132064,
    },
    3298882407011454303: KwSearchHit {
        title: "Unknown",
        content: "The modern city has spread from the island (the Île de la Cité) and far beyond both banks of the Seine.",
        score: 4.108218,
    },
    5068393704069168579: KwSearchHit {
        title: "Unknown",
        content: "France has long been a highly centralized country, and Paris has come to be identified with a powerful central state, drawing to itself much of the",
        score: 3.2115037,
    },
    16545325271760060286: KwSearchHit {
        title: "Unknown",
        content: "Paris, city and capital of France, situated in the north-central part of the country.",
        score: 4.66901,
    },
    14397306707317103710: KwSearchHit {
        title: "Unknown",
        content: "People were living on the site of the present-day city, located along the Seine River some 233 miles (375 km) upstream from the river's mouth on the",
        score: 8.916491,
    },
} - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.674272Z  INFO ThreadId(04) src/rag.rs:364: kw_scores: {
    14397306707317103710: 1.0,
    16545325271760060286: 0.25547932420463054,
    5068393704069168579: 0.0,
    17408515414919045690: 0.3366458502019802,
    3298882407011454303: 0.15718077058646557,
} - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.674285Z  INFO ThreadId(04) src/rag.rs:373: final_scores: {
    17408515414919045690: 0.1683229251009901,
    5068393704069168579: 0.0,
    656120371065341945: 0.0,
    16545325271760060286: 0.6277396621023152,
    3298882407011454303: 0.07859038529323278,
    13314534164983550839: 0.20931111275750988,
    14397306707317103710: 0.8848429117438126,
    8064019955072080751: 0.159751936092601,
} - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.674295Z  INFO ThreadId(04) src/rag.rs:384: final_ranking: [
    (
        14397306707317103710,
        0.8848429117438126,
    ),
    (
        16545325271760060286,
        0.6277396621023152,
    ),
    (
        13314534164983550839,
        0.20931111275750988,
    ),
    (
        17408515414919045690,
        0.1683229251009901,
    ),
    (
        8064019955072080751,
        0.159751936092601,
    ),
    (
        3298882407011454303,
        0.07859038529323278,
    ),
    (
        5068393704069168579,
        0.0,
    ),
    (
        656120371065341945,
        0.0,
    ),
] - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.674331Z  INFO ThreadId(04) src/rag.rs:428: retrieved: [
    RagScoredPoint {
        source: "People were living on the site of the present-day city, located along the Seine River some 233 miles (375 km) upstream from the river's mouth on the",
        score: 0.8848429117438126,
    },
    RagScoredPoint {
        source: "Paris, city and capital of France, situated in the north-central part of the country.",
        score: 0.6277396621023152,
    },
    RagScoredPoint {
        source: "Paris occupies a central position in the rich agricultural region known as the Paris Basin, and it constitutes one of eight départements of the Île-de",
        score: 0.20931111275750988,
    },
    RagScoredPoint {
        source: "-France administrative region. It is by far the country's most important centre of commerce and culture.",
        score: 0.1683229251009901,
    },
    RagScoredPoint {
        source: "Paris's site at a crossroads of both water and land routes significant not only to France but also to Europe has had a continuing influence on its",
        score: 0.159751936092601,
    },
    RagScoredPoint {
        source: "The modern city has spread from the island (the Île de la Cité) and far beyond both banks of the Seine.",
        score: 0.07859038529323278,
    },
    RagScoredPoint {
        source: "France has long been a highly centralized country, and Paris has come to be identified with a powerful central state, drawing to itself much of the",
        score: 0.0,
    },
    RagScoredPoint {
        source: "For centuries Paris has been one of the world's most important and attractive cities.",
        score: 0.0,
    },
] - request_id: 9ef0f67e-da44-444a-acba-b5f8ff9ec830
2025-05-20T09:25:14.674379Z  INFO ThreadId(04) src/rag.rs:1054: rag_policy: last-user-message
2025-05-20T09:25:14.674383Z  INFO ThreadId(04) src/rag.rs:1057: context:
People were living on the site of the present-day city, located along the Seine River some 233 miles (375 km) upstream from the river's mouth on the

Paris, city and capital of France, situated in the north-central part of the country.

Paris occupies a central position in the rich agricultural region known as the Paris Basin, and it constitutes one of eight départements of the Île-de

-France administrative region. It is by far the country's most important centre of commerce and culture.

Paris's site at a crossroads of both water and land routes significant not only to France but also to Europe has had a continuing influence on its

The modern city has spread from the island (the Île de la Cité) and far beyond both banks of the Seine.

France has long been a highly centralized country, and Paris has come to be identified with a powerful central state, drawing to itself much of the

For centuries Paris has been one of the world's most important and attractive cities.
2025-05-20T09:25:14.674433Z  INFO ThreadId(04) src/rag.rs:1131: Merge RAG context into last user message.

(more logs)
```

</details>
