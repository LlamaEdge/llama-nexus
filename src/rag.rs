use crate::{
    config::McpToolServerConfig,
    dual_debug, dual_error, dual_info, dual_warn,
    error::{ServerError, ServerResult},
    mcp::{
        MCP_KEYWORD_SEARCH_CLIENT, MCP_VECTOR_SEARCH_CLIENT, USER_TO_MCP_CLIENTS, USER_TO_MCP_TOOLS,
    },
    server::{RoutingPolicy, ServerKind},
    AppState,
};
use axum::{
    extract::{Extension, State},
    http::HeaderMap,
    Json,
};
use chat_prompts::{error as ChatPromptsError, MergeRagContext, MergeRagContextPolicy};
use endpoints::{
    chat::{
        ChatCompletionObject, ChatCompletionRequest, ChatCompletionRequestBuilder,
        ChatCompletionRequestMessage, ChatCompletionUserMessageContent, Tool, ToolCall, ToolChoice,
        ToolFunction,
    },
    embeddings::{EmbeddingObject, EmbeddingRequest, EmbeddingsResponse, InputText},
    rag::vector_search::{DataFrom, RagScoredPoint, RetrieveObject},
};
use gaia_elastic_mcp_common::SearchResponse;
use gaia_kwsearch_mcp_common::{KwSearchHit, SearchDocumentsResponse};
use gaia_qdrant_mcp_common::{
    CreateCollectionResponse, Point, ScoredPoint, SearchPointsResponse, UpsertPointsResponse,
};
use gaia_tidb_mcp_common::TidbSearchResponse;
use rmcp::model::CallToolRequestParam;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{hash_map::DefaultHasher, HashMap, HashSet},
    fmt,
    hash::{Hash, Hasher},
    sync::Arc,
};
use text_splitter::{MarkdownSplitter, TextSplitter};
use tokio_util::sync::CancellationToken;
use tracing::info;

const DEFAULT_FILTER_LIMIT: u64 = 10;
const DEFAULT_FILTER_SCORE_THRESHOLD: f32 = 0.5;
const DEFAULT_FILTER_WEIGHTED_ALPHA: f64 = 0.5;

pub async fn chat_new(
    State(state): State<Arc<AppState>>,
    Extension(cancel_token): Extension<CancellationToken>,
    headers: HeaderMap,
    Json(mut chat_request): Json<ChatCompletionRequest>,
    request_id: impl AsRef<str>,
) -> ServerResult<axum::response::Response> {
    let request_id = request_id.as_ref();

    // * filter parameters
    let filter_limit = match chat_request.limit {
        Some(limit) => limit,
        None => DEFAULT_FILTER_LIMIT,
    };
    dual_debug!(
        "filter_limit: {} - request_id: {}",
        filter_limit,
        request_id
    );
    let filter_score_threshold = match chat_request.score_threshold {
        Some(score_threshold) => score_threshold,
        None => DEFAULT_FILTER_SCORE_THRESHOLD,
    };
    dual_debug!(
        "filter_score_threshold: {} - request_id: {}",
        filter_score_threshold,
        request_id
    );
    let weighted_alpha = match chat_request.weighted_alpha {
        Some(weighted_alpha) => weighted_alpha,
        None => DEFAULT_FILTER_WEIGHTED_ALPHA,
    };
    dual_debug!(
        "weighted_alpha: {} - request_id: {}",
        weighted_alpha,
        request_id
    );

    // Get the last user message text
    let query_text = match chat_request.messages.last() {
        Some(ChatCompletionRequestMessage::User(user_message)) => match user_message.content() {
            ChatCompletionUserMessageContent::Text(text) => text.clone(),
            _ => {
                let err_msg = "The last message in the request is not a text-only user message";
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::BadRequest(err_msg.to_string()));
            }
        },
        _ => {
            let err_msg = "The last message in the request is not a user message";
            dual_error!("{} - request_id: {}", err_msg, request_id);
            return Err(ServerError::BadRequest(err_msg.to_string()));
        }
    };

    // // Get qdrant configs
    // dual_info!(
    //     "Parsing parameters for vector search - request_id: {}",
    //     request_id
    // );
    // let qdrant_config_vec = match get_qdrant_configs(
    //     &chat_request,
    //     filter_limit,
    //     filter_score_threshold,
    //     &request_id,
    // )
    // .await
    // {
    //     Ok(configs) => configs,
    //     Err(e) => {
    //         let err_msg = format!("Failed to get the VectorDB config: {e}");
    //         dual_error!(
    //             "Failed to get the VectorDB config: {} - request_id: {}",
    //             e,
    //             request_id
    //         );
    //         return Err(ServerError::Operation(err_msg));
    //     }
    // };

    // // Parallel execution of keyword search and vector search
    // let (res_kw_search, res_vector_search) = tokio::join!(
    //     perform_keyword_search_new(
    //         State(state.clone()),
    //         &query_text,
    //         &chat_request,
    //         // filter_limit,
    //         &request_id
    //     ),
    //     perform_vector_search(
    //         State(state.clone()),
    //         Extension(cancel_token.clone()),
    //         headers.clone(),
    //         &chat_request,
    //         &request_id
    //     )
    // );

    // let kw_hits = res_kw_search.unwrap();
    // let vector_hits = res_vector_search.unwrap();

    // vector search
    dual_info!("Performing vector search - request_id: {}", request_id);
    let mut vector_hits = Vec::new();
    if MCP_VECTOR_SEARCH_CLIENT.get().is_some() {
        vector_hits = perform_vector_search(
            State(state.clone()),
            Extension(cancel_token.clone()),
            headers.clone(),
            &chat_request,
            request_id,
        )
        .await?;
        dual_info!(
            "Retrieved {} points from the vector search - request_id: {}",
            vector_hits.len(),
            request_id
        );
    } else {
        dual_info!(
            "Ignore vector search: No vector mcp server available - request_id: {}",
            request_id
        );
    }

    // keyword search
    dual_info!(
        "Performing agentic keyword search - request_id: {}",
        request_id
    );
    let mut kw_hits = Vec::new();
    if MCP_KEYWORD_SEARCH_CLIENT.get().is_some() {
        kw_hits = perform_keyword_search_new(
            State(state.clone()),
            &query_text,
            &chat_request,
            &request_id,
        )
        .await?;
        dual_info!(
            "Retrieved {} hits from the keyword search - request_id: {}",
            kw_hits.len(),
            request_id
        );
    } else {
        dual_info!(
            "Ignore keyword search: No keyword search mcp server available - request_id: {}",
            request_id
        );
    }

    // * rerank
    let hits = {
        // create a hash map from kw_hits: key is the hash value of the content of the hit, value is the hit
        let mut map_kwsearch_hits = HashMap::new();
        let mut scores_kwsearch_hits = HashMap::new();
        if !kw_hits.is_empty() {
            for hit in kw_hits {
                let hash_value = calculate_hash(&hit.content);
                scores_kwsearch_hits.insert(hash_value, hit.score);
                map_kwsearch_hits.insert(hash_value, hit);
            }

            // normalize the kw_scores
            scores_kwsearch_hits = min_max_normalize(&scores_kwsearch_hits);

            dual_debug!(
                "kw_scores: {:#?} - request_id: {}",
                &scores_kwsearch_hits,
                request_id
            );
        }

        // create a hash map from retrieve_object_vec: key is the hash value of the source of the point, value is the point
        let mut map_vector_search_hits = HashMap::new();
        let mut scores_vector_search_hits = HashMap::new();
        if !vector_hits.is_empty() {
            let points = vector_hits[0].points.as_ref().unwrap().clone();
            if !points.is_empty() {
                for point in points {
                    let hash_value = calculate_hash(&point.source);
                    scores_vector_search_hits.insert(hash_value, point.score);
                    map_vector_search_hits.insert(hash_value, point);
                }

                // normalize the em_scores
                scores_vector_search_hits = min_max_normalize(&scores_vector_search_hits);

                dual_debug!(
                    "em_scores: {:#?} - request_id: {}",
                    &scores_vector_search_hits,
                    request_id
                );
            }
        }

        // fuse the two hash maps
        dual_info!(
            "Fusing vector and keyword search results - request_id: {}",
            request_id
        );
        let fused_scores = weighted_fusion(
            scores_kwsearch_hits,
            scores_vector_search_hits,
            weighted_alpha,
        );

        if !fused_scores.is_empty() {
            dual_debug!(
                "final_scores: {:#?} - request_id: {}",
                &fused_scores,
                request_id
            );

            // Sort by score from high to low
            dual_info!(
                "Re-ranking the fused search results - request_id: {}",
                request_id
            );
            let mut final_ranking: Vec<(u64, f64)> = fused_scores.into_iter().collect();
            final_ranking.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            // if final_ranking.len() > filter_limit as usize {
            //     final_ranking.truncate(filter_limit as usize);
            // }

            let mut retrieved = Vec::new();
            for (hash_value, score) in final_ranking.iter() {
                if map_kwsearch_hits.contains_key(hash_value) {
                    retrieved.push(RagScoredPoint {
                        source: map_kwsearch_hits[hash_value].content.clone(),
                        score: *score,
                        from: DataFrom::KeywordSearch,
                    });
                } else if map_vector_search_hits.contains_key(hash_value) {
                    retrieved.push(RagScoredPoint {
                        source: map_vector_search_hits[hash_value].source.clone(),
                        score: *score,
                        from: DataFrom::VectorSearch,
                    });
                }
            }

            let retrieve_object = RetrieveObject {
                limit: retrieved.len(),
                score_threshold: filter_score_threshold,
                points: Some(retrieved),
            };

            vec![retrieve_object]
        } else {
            dual_warn!("No point retrieved - request_id: {}", request_id);

            vec![]
        }
    };

    dual_debug!(
        "Retrieved {} points in total - request_id: {}",
        hits.len(),
        request_id
    );

    // * generate context
    dual_info!("Generating context - request_id: {}", request_id);
    let mut context = String::new();
    if !hits.is_empty() {
        for retrieve_object in hits.iter() {
            match retrieve_object.points.as_ref() {
                Some(scored_points) => {
                    match scored_points.is_empty() {
                        false => {
                            for (idx, point) in scored_points.iter().enumerate() {
                                // log
                                dual_debug!(
                                    "request_id: {} - Point-{}, score: {}, source: {}",
                                    request_id,
                                    idx,
                                    point.score,
                                    &point.source
                                );

                                context.push_str(&point.source);
                                context.push_str("\n\n");
                            }
                        }
                        true => {
                            // log
                            dual_warn!(
                                "No search results used as context - request_id: {}",
                                request_id
                            );
                        }
                    }
                }
                None => {
                    // log
                    dual_warn!(
                        "No search results used as context - request_id: {}",
                        request_id
                    );
                }
            }
        }
    } else {
        context = "No context retrieved".to_string();
    }
    dual_debug!("request_id: {} - context:\n{}", request_id, context);

    // * merge context into chat request
    dual_info!(
        "Merging context into chat request - request_id: {}",
        request_id
    );
    if chat_request.messages.is_empty() {
        let err_msg = "Found empty chat messages";

        // log
        dual_error!("{} - request_id: {}", err_msg, request_id);

        return Err(ServerError::BadRequest(err_msg.to_string()));
    }
    // get the prompt template from the chat server
    let prompt_template = {
        let server_info = state.server_info.read().await;
        let chat_server = server_info
            .servers
            .iter()
            .find(|(_server_id, server)| server.chat_model.is_some());
        match chat_server {
            Some((_server_id, chat_server)) => {
                let chat_model = chat_server.chat_model.as_ref().unwrap();
                chat_model.prompt_template.unwrap()
            }
            None => {
                let err_msg = "No chat server available";
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg.to_string()));
            }
        }
    };
    // get the rag policy
    let (rag_policy, rag_prompt) = {
        let config = state.config.read().await;
        (config.rag.policy, config.rag.prompt.clone())
    };
    if let Err(e) = RagPromptBuilder::build(
        &mut chat_request.messages,
        &[context],
        prompt_template.has_system_prompt(),
        rag_policy,
        rag_prompt,
    ) {
        let err_msg = e.to_string();

        // log
        dual_error!("{} - request_id: {}", err_msg, request_id);

        return Err(ServerError::Operation(err_msg));
    }

    // * perform chat completion
    dual_info!("Performing chat completion - request_id: {}", request_id);
    crate::handlers::chat(
        State(state.clone()),
        Extension(cancel_token.clone()),
        headers,
        Json(chat_request),
        &request_id,
    )
    .await
}

async fn perform_keyword_search_new(
    State(state): State<Arc<AppState>>,
    query: impl AsRef<str>,
    chat_request: &ChatCompletionRequest,
    // filter_limit: u64,
    request_id: impl AsRef<str>,
) -> ServerResult<Vec<KwSearchHit>> {
    let request_id = request_id.as_ref();

    // get the user id from the request
    let user_id = match chat_request.user.as_ref() {
        Some(user_id) => user_id,
        None => {
            let err_msg = "User ID is not found in the request";
            dual_error!("{} - request_id: {}", err_msg, request_id);
            return Err(ServerError::Operation(err_msg.to_string()));
        }
    };

    match MCP_KEYWORD_SEARCH_CLIENT.get() {
        Some(mcp_client) => {
            // get mcp tools from keyword search mcp server
            let mcp_tool_list = mcp_client
                .read()
                .await
                .raw
                .peer()
                .list_tools(Default::default())
                .await
                .map_err(|e| {
                    let err_msg = format!("Failed to list tools: {e}");
                    dual_error!("{}", &err_msg);
                    ServerError::Operation(err_msg)
                })?;

            // convert mcp tools to llama tools
            let mut llama_tools: Vec<Tool> = Vec::new();
            mcp_tool_list.tools.iter().for_each(|rmcp_tool| {
                let tool_name = rmcp_tool.name.to_string();
                let tool = Tool::new(ToolFunction {
                    name: tool_name.clone(),
                    description: rmcp_tool.description.as_ref().map(|s| s.to_string()),
                    parameters: Some((*rmcp_tool.input_schema).clone()),
                });

                llama_tools.push(tool.clone());
            });

            let text = query.as_ref();
            // let user_prompt  = format!(
            //     "Extract the keywords from the following text. Avoid stop words, filler words, or overly generic terms (e.g., “how”, “can”, “thing”, “way”). The keywords should be separated by spaces.\n\nText: {text:#?}",
            // );
            let user_prompt  = format!(
                    "Please extract 3 to 5 keywords from my question, separated by spaces. Then, try to return a tool call that invokes the keyword search tool.\n\nMy question is: {text:#?}",
                );

            let user_message = ChatCompletionRequestMessage::new_user_message(
                ChatCompletionUserMessageContent::Text(user_prompt),
                None,
            );

            // create a request
            let request = ChatCompletionRequestBuilder::new(&[user_message])
                .with_tools(llama_tools)
                .with_tool_choice(ToolChoice::Auto)
                .with_user(user_id)
                .build();

            dual_debug!(
                "request for getting keywords:\n{} - request_id: {}",
                serde_json::to_string_pretty(&request).unwrap(),
                request_id
            );

            // get the chat server
            let target_server_info = {
                let servers = state.server_group.read().await;
                let chat_servers = match servers.get(&ServerKind::chat) {
                    Some(servers) => servers,
                    None => {
                        let err_msg = "No chat server available";
                        dual_error!("{} - request_id: {}", err_msg, request_id);
                        return Err(ServerError::Operation(err_msg.to_string()));
                    }
                };

                match chat_servers.next().await {
                    Ok(target_server_info) => target_server_info,
                    Err(e) => {
                        let err_msg = format!("Failed to get the chat server: {e}");
                        dual_error!("{} - request_id: {}", err_msg, request_id);
                        return Err(ServerError::Operation(err_msg));
                    }
                }
            };

            let chat_service_url = format!(
                "{}/v1/chat/completions",
                target_server_info.url.trim_end_matches('/')
            );
            dual_debug!(
                "Forward the chat request to {} - request_id: {}",
                chat_service_url,
                request_id
            );

            // Create a request client
            let response = reqwest::Client::new()
                .post(&chat_service_url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(&request)
                .send()
                .await
                .map_err(|e| {
                    let err_msg = format!("Failed to send the chat request: {e}");
                    dual_error!("{} - request_id: {}", err_msg, request_id);
                    ServerError::Operation(err_msg)
                })?;

            let status = response.status();
            dual_debug!("status: {} - request_id: {}", status, request_id);
            let headers = response.headers().clone();

            // check if the response has a header with the key "requires-tool-call"
            if let Some(value) = headers.get("requires-tool-call") {
                // convert the value to a boolean
                let requires_tool_call: bool = value.to_str().unwrap().parse().unwrap();

                dual_debug!(
                    "requires_tool_call: {} - request_id: {}",
                    requires_tool_call,
                    request_id
                );

                if requires_tool_call {
                    let bytes = response.bytes().await.map_err(|e| {
                        let err_msg = format!("Failed to get the response bytes: {e}");
                        dual_error!("{} - request_id: {}", err_msg, request_id);
                        ServerError::Operation(err_msg)
                    })?;

                    let chat_completion: ChatCompletionObject = match serde_json::from_slice(&bytes)
                    {
                        Ok(completion) => completion,
                        Err(e) => {
                            let err_msg = format!("Failed to parse the response: {e}");
                            dual_error!("{} - request_id: {}", err_msg, request_id);
                            return Err(ServerError::Operation(err_msg));
                        }
                    };

                    let assistant_message = &chat_completion.choices[0].message;

                    match call_keyword_search_mcp_server_new(
                        assistant_message.tool_calls.as_slice(),
                        &request_id,
                    )
                    .await
                    {
                        Ok(kw_hits) => return Ok(kw_hits),
                        Err(ServerError::McpNotFoundClient) => {
                            dual_warn!("Not found MCP server - request_id: {}", request_id);
                            return Ok(vec![]);
                        }
                        Err(e) => {
                            let err_msg = format!(
                                "Failed to call MCP server: {e} - request_id: {request_id}"
                            );
                            dual_error!("{}", err_msg);
                            return Err(ServerError::Operation(err_msg));
                        }
                    }
                }
            }
        }
        None => {
            let warn_msg = "No keyword search mcp server connected";
            dual_warn!("{} - request_id: {}", warn_msg, request_id);
        }
    }

    Ok(vec![])
}

async fn _perform_keyword_search_new_old(
    State(state): State<Arc<AppState>>,
    query: impl AsRef<str>,
    chat_request: &mut ChatCompletionRequest,
    // filter_limit: u64,
    request_id: impl AsRef<str>,
) -> ServerResult<Vec<KwSearchHit>> {
    let request_id = request_id.as_ref();

    // get the user id from the request
    let user_id = match chat_request.user.as_ref() {
        Some(user_id) => user_id,
        None => {
            let err_msg = "User ID is not found in the request";
            dual_error!("{} - request_id: {}", err_msg, request_id);
            return Err(ServerError::Operation(err_msg.to_string()));
        }
    };

    dual_debug!(
        "chat_request:\n{}",
        serde_json::to_string_pretty(chat_request).unwrap()
    );

    // load tools from the `mcp_tools` field in the request
    if let Some(config) = &chat_request.kw_search_mcp_tool {
        dual_debug!("mcp server info: {:?}", config);

        let mut server_config = McpToolServerConfig {
            name: config.server_label.clone(),
            transport: config.transport,
            url: config.server_url.clone(),
            enable: true,
            tools: None,
        };

        // connect the mcp server and get the tools
        server_config
            .connect_mcp_server_by_user(chat_request.user.as_ref().unwrap())
            .await?;

        // get the allowed tools
        let allowed_tools = config.allowed_tools.as_deref().unwrap_or(&[]);

        // get the tools from the mcp server
        if let Some(tools) = server_config.tools.as_deref() {
            let mut tools_from_kwsearch = Vec::new();
            tools.iter().for_each(|rmcp_tool| {
                let tool_name = rmcp_tool.name.to_string();
                let tool = Tool::new(ToolFunction {
                    name: tool_name.clone(),
                    description: rmcp_tool.description.as_ref().map(|s| s.to_string()),
                    parameters: Some((*rmcp_tool.input_schema).clone()),
                });

                if allowed_tools.is_empty() || allowed_tools.contains(&tool_name) {
                    dual_debug!("tool to be added: {:?}", &tool);
                    tools_from_kwsearch.push(tool.clone());
                }
            });

            let text = query.as_ref();
            // let user_prompt  = format!(
            //     "Extract the keywords from the following text. Avoid stop words, filler words, or overly generic terms (e.g., “how”, “can”, “thing”, “way”). The keywords should be separated by spaces.\n\nText: {text:#?}",
            // );
            let user_prompt  = format!(
                "Please extract 3 to 5 keywords from my question, separated by spaces. Then, try to return a tool call that invokes the keyword search tool.\n\nMy question is: {text:#?}",
            );

            let user_message = ChatCompletionRequestMessage::new_user_message(
                ChatCompletionUserMessageContent::Text(user_prompt),
                None,
            );

            // create a request
            let mut request = ChatCompletionRequestBuilder::new(&[user_message])
                .with_tools(tools_from_kwsearch)
                .with_tool_choice(ToolChoice::Auto)
                .with_user(user_id)
                .build();

            dual_debug!(
                "request for getting keywords:\n{} - request_id: {}",
                serde_json::to_string_pretty(&request).unwrap(),
                request_id
            );

            // get the chat server
            let target_server_info = {
                let servers = state.server_group.read().await;
                let chat_servers = match servers.get(&ServerKind::chat) {
                    Some(servers) => servers,
                    None => {
                        let err_msg = "No chat server available";
                        dual_error!("{} - request_id: {}", err_msg, request_id);
                        return Err(ServerError::Operation(err_msg.to_string()));
                    }
                };

                match chat_servers.next().await {
                    Ok(target_server_info) => target_server_info,
                    Err(e) => {
                        let err_msg = format!("Failed to get the chat server: {e}");
                        dual_error!("{} - request_id: {}", err_msg, request_id);
                        return Err(ServerError::Operation(err_msg));
                    }
                }
            };

            let chat_service_url = format!(
                "{}/v1/chat/completions",
                target_server_info.url.trim_end_matches('/')
            );
            dual_debug!(
                "Forward the chat request to {} - request_id: {}",
                chat_service_url,
                request_id
            );

            // Create a request client
            let response = reqwest::Client::new()
                .post(&chat_service_url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(&request)
                .send()
                .await
                .map_err(|e| {
                    let err_msg = format!("Failed to send the chat request: {e}");
                    dual_error!("{} - request_id: {}", err_msg, request_id);
                    ServerError::Operation(err_msg)
                })?;

            let status = response.status();
            dual_debug!("status: {} - request_id: {}", status, request_id);
            let headers = response.headers().clone();

            // check if the response has a header with the key "requires-tool-call"
            if let Some(value) = headers.get("requires-tool-call") {
                // convert the value to a boolean
                let requires_tool_call: bool = value.to_str().unwrap().parse().unwrap();

                dual_debug!(
                    "requires_tool_call: {} - request_id: {}",
                    requires_tool_call,
                    request_id
                );

                if requires_tool_call {
                    let bytes = response.bytes().await.map_err(|e| {
                        let err_msg = format!("Failed to get the response bytes: {e}");
                        dual_error!("{} - request_id: {}", err_msg, request_id);
                        ServerError::Operation(err_msg)
                    })?;

                    let chat_completion: ChatCompletionObject = match serde_json::from_slice(&bytes)
                    {
                        Ok(completion) => completion,
                        Err(e) => {
                            let err_msg = format!("Failed to parse the response: {e}");
                            dual_error!("{} - request_id: {}", err_msg, request_id);
                            return Err(ServerError::Operation(err_msg));
                        }
                    };

                    let assistant_message = &chat_completion.choices[0].message;

                    match _call_keyword_search_mcp_server_new_old(
                        assistant_message.tool_calls.as_slice(),
                        &mut request,
                        &request_id,
                    )
                    .await
                    {
                        Ok(kw_hits) => return Ok(kw_hits),
                        Err(ServerError::McpNotFoundClient) => {
                            dual_warn!("Not found MCP server - request_id: {}", request_id);
                            return Ok(vec![]);
                        }
                        Err(e) => {
                            let err_msg = format!(
                                "Failed to call MCP server: {e} - request_id: {request_id}"
                            );
                            dual_error!("{}", err_msg);
                            return Err(ServerError::Operation(err_msg));
                        }
                    }
                }
            }
        }
    }

    // erase mcp tools from USER_TO_MCP_TOOLS by user id
    if let Some(user_to_mcp_tools) = USER_TO_MCP_TOOLS.get() {
        let mut user_to_mcp_tools = user_to_mcp_tools.write().await;
        user_to_mcp_tools.remove(user_id);

        dual_debug!(
            "Erase mcp tools from USER_TO_MCP_TOOLS by user id: {} - request_id: {}",
            user_id,
            request_id
        );
    }

    // erase mcp clients from USER_TO_MCP_CLIENTS by user id
    if let Some(user_to_mcp_clients) = USER_TO_MCP_CLIENTS.get() {
        let mut user_to_mcp_clients = user_to_mcp_clients.write().await;
        user_to_mcp_clients.remove(user_id);

        dual_debug!(
            "Erase mcp clients from USER_TO_MCP_CLIENTS by user id: {} - request_id: {}",
            user_id,
            request_id
        );
    }

    Ok(vec![])
}

pub async fn _chat_old(
    State(state): State<Arc<AppState>>,
    Extension(cancel_token): Extension<CancellationToken>,
    headers: HeaderMap,
    Json(mut chat_request): Json<ChatCompletionRequest>,
) -> ServerResult<axum::response::Response> {
    let request_id = headers
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    dual_info!("Received a new chat request - request_id: {}", request_id);

    // * filter parameters
    let filter_limit = match chat_request.limit {
        Some(limit) => limit,
        None => DEFAULT_FILTER_LIMIT,
    };
    dual_debug!(
        "filter_limit: {} - request_id: {}",
        filter_limit,
        request_id
    );
    let filter_score_threshold = match chat_request.score_threshold {
        Some(score_threshold) => score_threshold,
        None => DEFAULT_FILTER_SCORE_THRESHOLD,
    };
    dual_debug!(
        "filter_score_threshold: {} - request_id: {}",
        filter_score_threshold,
        request_id
    );
    let weighted_alpha = match chat_request.weighted_alpha {
        Some(weighted_alpha) => weighted_alpha,
        None => DEFAULT_FILTER_WEIGHTED_ALPHA,
    };
    dual_debug!(
        "weighted_alpha: {} - request_id: {}",
        weighted_alpha,
        request_id
    );

    // Get the last user message text
    let query_text = match chat_request.messages.last() {
        Some(ChatCompletionRequestMessage::User(user_message)) => match user_message.content() {
            ChatCompletionUserMessageContent::Text(text) => text.clone(),
            _ => {
                let err_msg = "The last message in the request is not a text-only user message";
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::BadRequest(err_msg.to_string()));
            }
        },
        _ => {
            let err_msg = "The last message in the request is not a user message";
            dual_error!("{} - request_id: {}", err_msg, request_id);
            return Err(ServerError::BadRequest(err_msg.to_string()));
        }
    };

    // Get qdrant configs
    let qdrant_config_vec = match _get_qdrant_configs(
        &chat_request,
        filter_limit,
        filter_score_threshold,
        &request_id,
    )
    .await
    {
        Ok(configs) => configs,
        Err(e) => {
            let err_msg = format!("Failed to get the VectorDB config: {e}");
            dual_error!(
                "Failed to get the VectorDB config: {} - request_id: {}",
                e,
                request_id
            );
            return Err(ServerError::Operation(err_msg));
        }
    };

    // Parallel execution of keyword search and vector search
    let (res_kw_search, res_vector_search) = tokio::join!(
        _perform_keyword_search_old(
            State(state.clone()),
            &query_text,
            &chat_request,
            filter_limit,
            &request_id
        ),
        perform_vector_search(
            State(state.clone()),
            Extension(cancel_token.clone()),
            headers.clone(),
            &chat_request,
            &request_id
        )
    );

    // Handle results
    let kw_hits = res_kw_search?;
    let vector_hits = res_vector_search?;

    // * rerank
    let hits = {
        // create a hash map from kw_hits: key is the hash value of the content of the hit, value is the hit
        let mut kw_hits_map = HashMap::new();
        let mut kw_scores = HashMap::new();
        if !kw_hits.is_empty() {
            for hit in kw_hits {
                let hash_value = calculate_hash(&hit.content);
                kw_scores.insert(hash_value, hit.score);
                kw_hits_map.insert(hash_value, hit);
            }

            dual_info!(
                "kw_hits_map: {:#?} - request_id: {}",
                &kw_hits_map,
                request_id
            );

            // normalize the kw_scores
            let kw_scores = min_max_normalize(&kw_scores);

            dual_info!("kw_scores: {:#?} - request_id: {}", &kw_scores, request_id);
        }

        // create a hash map from retrieve_object_vec: key is the hash value of the source of the point, value is the point
        let mut em_hits_map = HashMap::new();
        let mut em_scores = HashMap::new();
        if !vector_hits.is_empty() {
            let points = vector_hits[0].points.as_ref().unwrap().clone();
            if !points.is_empty() {
                for point in points {
                    let hash_value = calculate_hash(&point.source);
                    em_scores.insert(hash_value, point.score);
                    em_hits_map.insert(hash_value, point);
                }

                dual_info!(
                    "em_hits_map: {:#?} - request_id: {}",
                    &em_hits_map,
                    request_id
                );

                // normalize the em_scores
                let em_scores = min_max_normalize(&em_scores);

                dual_info!("em_scores: {:#?} - request_id: {}", &em_scores, request_id);
            }
        }

        // fuse the two hash maps
        let fused_scores = weighted_fusion(kw_scores, em_scores, weighted_alpha);

        if !fused_scores.is_empty() {
            dual_debug!(
                "final_scores: {:#?} - request_id: {}",
                &fused_scores,
                request_id
            );

            // Sort by score from high to low
            let mut final_ranking: Vec<(u64, f64)> = fused_scores.into_iter().collect();
            final_ranking.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            // if final_ranking.len() > filter_limit as usize {
            //     final_ranking.truncate(filter_limit as usize);
            // }

            // Print final ranking
            dual_debug!(
                "final_ranking: {:#?} - request_id: {}",
                &final_ranking,
                request_id
            );

            let mut retrieved = Vec::new();
            for (hash_value, score) in final_ranking.iter() {
                if kw_hits_map.contains_key(hash_value) {
                    retrieved.push(RagScoredPoint {
                        source: kw_hits_map[hash_value].content.clone(),
                        score: *score,
                        from: DataFrom::KeywordSearch,
                    });
                } else if em_hits_map.contains_key(hash_value) {
                    retrieved.push(RagScoredPoint {
                        source: em_hits_map[hash_value].source.clone(),
                        score: *score,
                        from: DataFrom::VectorSearch,
                    });
                }
            }

            dual_info!("retrieved: {:#?} - request_id: {}", &retrieved, request_id);

            let retrieve_object = RetrieveObject {
                limit: retrieved.len(),
                score_threshold: filter_score_threshold,
                points: Some(retrieved),
            };

            vec![retrieve_object]
        } else {
            dual_warn!("No point retrieved - request_id: {}", request_id);

            vec![]
        }
    };

    // * generate context
    let mut context = String::new();
    if !hits.is_empty() {
        for (idx, retrieve_object) in hits.iter().enumerate() {
            match retrieve_object.points.as_ref() {
                Some(scored_points) => {
                    match scored_points.is_empty() {
                        false => {
                            for (idx, point) in scored_points.iter().enumerate() {
                                // log
                                dual_debug!(
                                    "request_id: {} - Point-{}, score: {}, source: {}",
                                    request_id,
                                    idx,
                                    point.score,
                                    &point.source
                                );

                                context.push_str(&point.source);
                                context.push_str("\n\n");
                            }
                        }
                        true => {
                            // log
                            dual_warn!("No point retrieved from the collection `{}` (score < threshold {}) - request_id: {}", qdrant_config_vec[idx].collection_name, qdrant_config_vec[idx].score_threshold, request_id);
                        }
                    }
                }
                None => {
                    // log
                    dual_warn!("No point retrieved from the collection `{}` (score < threshold {}) - request_id: {}", qdrant_config_vec[idx].collection_name, qdrant_config_vec[idx].score_threshold, request_id);
                }
            }
        }
        dual_debug!("request_id: {} - context:\n{}", request_id, context);
    } else {
        context = "No context retrieved".to_string();
    }

    // * merge context into chat request
    if chat_request.messages.is_empty() {
        let err_msg = "Found empty chat messages";

        // log
        dual_error!("{} - request_id: {}", err_msg, request_id);

        return Err(ServerError::BadRequest(err_msg.to_string()));
    }
    // get the prompt template from the chat server
    let prompt_template = {
        let server_info = state.server_info.read().await;
        let chat_server = server_info
            .servers
            .iter()
            .find(|(_server_id, server)| server.chat_model.is_some());
        match chat_server {
            Some((_server_id, chat_server)) => {
                let chat_model = chat_server.chat_model.as_ref().unwrap();
                chat_model.prompt_template.unwrap()
            }
            None => {
                let err_msg = "No chat server available";
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg.to_string()));
            }
        }
    };
    // get the rag policy
    let (rag_policy, rag_prompt) = {
        let config = state.config.read().await;
        (config.rag.policy, config.rag.prompt.clone())
    };
    if let Err(e) = RagPromptBuilder::build(
        &mut chat_request.messages,
        &[context],
        prompt_template.has_system_prompt(),
        rag_policy,
        rag_prompt,
    ) {
        let err_msg = e.to_string();

        // log
        dual_error!("{} - request_id: {}", err_msg, request_id);

        return Err(ServerError::Operation(err_msg));
    }

    // * perform chat completion
    crate::handlers::_chat_old(
        State(state.clone()),
        Extension(cancel_token.clone()),
        headers,
        Json(chat_request),
    )
    .await
}

async fn _perform_keyword_search_old(
    State(state): State<Arc<AppState>>,
    text: &str,
    chat_request: &ChatCompletionRequest,
    filter_limit: u64,
    request_id: &str,
) -> ServerResult<Vec<KwSearchHit>> {
    let mut kw_hits: Vec<KwSearchHit> = Vec::new();

    match MCP_KEYWORD_SEARCH_CLIENT.get() {
        Some(mcp_client) => {
            let mcp_name = mcp_client.read().await.name.clone();

            match mcp_name.as_str() {
                "gaia-keyword-search" => {
                    // extract keywords from the user message
                    let keywords =
                        _extract_keywords_by_llm(State(state.clone()), text, request_id).await?;

                    info!("Extracted keywords: {}", &keywords);

                    let kw_search_index = match chat_request.kw_search_index.as_ref() {
                        Some(index) if !index.is_empty() => index.to_string(),
                        _ => {
                            let err_msg = "Not found `kw_search_index` field in the request. `kw_search_index` field is required for kw-search-server. ";
                            dual_error!("{} - request_id: {}", err_msg, request_id);
                            return Err(ServerError::BadRequest(err_msg.to_string()));
                        }
                    };

                    // request param
                    let request_param = CallToolRequestParam {
                        name: "search_documents".into(),
                        arguments: Some(serde_json::Map::from_iter([
                            ("index_name".to_string(), Value::from(kw_search_index)),
                            ("query".to_string(), Value::from(keywords)),
                            ("limit".to_string(), Value::from(filter_limit)),
                        ])),
                    };

                    // call the search_documents tool
                    let tool_result = mcp_client
                        .read()
                        .await
                        .raw
                        .peer()
                        .call_tool(request_param)
                        .await
                        .map_err(|e| {
                            let err_msg = format!("Failed to call the tool: {e}");
                            dual_error!("{} - request_id: {}", err_msg, request_id);
                            ServerError::Operation(err_msg)
                        })?;

                    let search_response = SearchDocumentsResponse::from(tool_result);
                    kw_hits = search_response.hits;
                }
                "gaia-elastic-search" => {
                    let es_search_index = match chat_request.es_search_index.as_ref() {
                        Some(index) if !index.is_empty() => {
                            let index = index.clone();

                            dual_info!("Index name: {} - request_id: {}", &index, request_id);

                            index
                        }
                        _ => {
                            let err_msg = "Not found `es_search_index` field in the request. `es_search_index` field is required for Elasticsearch server. ";

                            dual_error!("{} - request_id: {}", err_msg, request_id);

                            return Err(ServerError::BadRequest(err_msg.to_string()));
                        }
                    };

                    // parse fields to search
                    let es_search_fields: Vec<Value> = match chat_request.es_search_fields.as_ref()
                    {
                        Some(fields) if !fields.is_empty() => {
                            let fields = fields
                                .iter()
                                .map(|f| serde_json::Value::String(f.clone()))
                                .collect();

                            dual_info!(
                                "Fields to search: {:?} - request_id: {}",
                                &fields,
                                request_id
                            );

                            fields
                        }
                        _ => {
                            let err_msg = "Not found `es_search_fields` field in the request. `es_search_fields` field is required for Elasticsearch server. ";

                            dual_error!("{} - request_id: {}", err_msg, request_id);

                            return Err(ServerError::BadRequest(err_msg.to_string()));
                        }
                    };

                    // request param
                    let request_param = CallToolRequestParam {
                        name: "search".into(),
                        arguments: Some(serde_json::Map::from_iter([
                            ("index".to_string(), Value::from(es_search_index)),
                            ("query".to_string(), Value::from(text.to_string())),
                            ("fields".to_string(), Value::Array(es_search_fields)),
                            ("size".to_string(), Value::from(filter_limit)),
                        ])),
                    };

                    // call tool
                    let tool_result = mcp_client
                        .read()
                        .await
                        .raw
                        .peer()
                        .call_tool(request_param)
                        .await
                        .map_err(|e| {
                            let err_msg = format!("Failed to call the tool: {e}");
                            dual_error!("{} - request_id: {}", err_msg, request_id);
                            ServerError::Operation(err_msg)
                        })?;

                    // parse tool result
                    let search_response = SearchResponse::from(tool_result);

                    if !search_response.hits.hits.is_empty() {
                        for hit in search_response.hits.hits.iter() {
                            let score = hit.score;
                            let title = hit
                                .source
                                .get("title")
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_string();
                            let content = hit
                                .source
                                .get("content")
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_string();

                            let kw_hit = KwSearchHit {
                                title,
                                content,
                                score,
                            };

                            kw_hits.push(kw_hit);
                        }
                    }
                }
                "gaia-tidb-search" => {
                    let keywords =
                        _extract_keywords_by_llm(State(state.clone()), text, request_id).await?;

                    info!("Extracted keywords: {}", &keywords);

                    let tidb_database = match chat_request.tidb_search_database.as_ref() {
                        Some(database) if !database.is_empty() => database.to_string(),
                        _ => {
                            let err_msg = "Not found `tidb_search_database` field in the request. `tidb_search_database` field is required for tidb-search-server. ";

                            dual_error!("{} - request_id: {}", err_msg, request_id);

                            return Err(ServerError::BadRequest(err_msg.to_string()));
                        }
                    };

                    let tidb_table_name = match chat_request.tidb_search_table.as_ref() {
                        Some(table_name) if !table_name.is_empty() => table_name.to_string(),
                        _ => {
                            let err_msg = "Not found `tidb_search_table` field in the request. `tidb_search_table` field is required for tidb-search-server. ";

                            dual_error!("{} - request_id: {}", err_msg, request_id);

                            return Err(ServerError::BadRequest(err_msg.to_string()));
                        }
                    };

                    // request param
                    let request_param = CallToolRequestParam {
                        name: "search".into(),
                        arguments: Some(serde_json::Map::from_iter([
                            (
                                "database".to_string(),
                                serde_json::Value::from(tidb_database),
                            ),
                            (
                                "table_name".to_string(),
                                serde_json::Value::from(tidb_table_name),
                            ),
                            ("limit".to_string(), serde_json::Value::from(filter_limit)),
                            ("query".to_string(), serde_json::Value::from(keywords)),
                        ])),
                    };

                    let tool_result = mcp_client
                        .read()
                        .await
                        .raw
                        .peer()
                        .call_tool(request_param)
                        .await
                        .map_err(|e| {
                            let err_msg = format!("Failed to call the tool: {e}");
                            dual_error!("{} - request_id: {}", err_msg, request_id);
                            ServerError::Operation(err_msg)
                        })?;

                    // parse tool result
                    let search_response = TidbSearchResponse::from(tool_result);

                    if !search_response.hits.is_empty() {
                        for hit in search_response.hits.iter() {
                            let kw_hit = KwSearchHit {
                                title: hit.title.clone(),
                                content: hit.content.clone(),
                                score: 0.0,
                            };

                            kw_hits.push(kw_hit);
                        }
                    }
                }
                _ => {
                    let err_msg = format!("Unsupported keyword search mcp server: {mcp_name}");
                    dual_error!("{} - request_id: {}", err_msg, request_id);
                    return Err(ServerError::Operation(err_msg));
                }
            }
        }
        None => {
            let warn_msg = "No keyword search mcp server connected";
            dual_warn!("{} - request_id: {}", warn_msg, request_id);
        }
    }

    Ok(kw_hits)
}

async fn perform_vector_search(
    State(state): State<Arc<AppState>>,
    Extension(cancel_token): Extension<CancellationToken>,
    headers: HeaderMap,
    chat_request: &ChatCompletionRequest,
    request_id: &str,
) -> ServerResult<Vec<RetrieveObject>> {
    retrieve_context_with_multiple_qdrant_configs(
        State(state),
        Extension(cancel_token),
        headers,
        request_id,
        chat_request,
    )
    .await
}

async fn _get_qdrant_configs(
    chat_request: &ChatCompletionRequest,
    limit: u64,
    score_threshold: f32,
    request_id: impl AsRef<str>,
) -> Result<Vec<QdrantConfig>, ServerError> {
    let request_id = request_id.as_ref();

    match chat_request.vdb_collection_name.as_deref() {
        Some(collection_name) if !collection_name.is_empty() => {
            dual_debug!(
                "Use the VectorDB settings from the request - request_id: {}",
                request_id
            );

            let collection_name_str = collection_name.join(",");

            dual_debug!(
                "collection name: {} - request_id: {}",
                collection_name_str,
                request_id
            );

            let mut qdrant_config_vec = vec![];
            for col_name in collection_name.iter() {
                qdrant_config_vec.push(QdrantConfig {
                    collection_name: col_name.to_string(),
                    limit,
                    score_threshold,
                });
            }

            Ok(qdrant_config_vec)
        }
        _ => {
            let err_msg = "The settings for vector search in the request are not correct. The `vdb_collection_name` field in the request should be provided.";

            dual_error!("{} - request_id: {}", err_msg, request_id);

            Err(ServerError::Operation(err_msg.into()))
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct QdrantConfig {
    pub(crate) collection_name: String,
    pub(crate) limit: u64,
    pub(crate) score_threshold: f32,
}
impl fmt::Display for QdrantConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "collection_name: {}, limit: {}, score_threshold: {}",
            self.collection_name, self.limit, self.score_threshold
        )
    }
}

async fn retrieve_context_with_multiple_qdrant_configs(
    State(state): State<Arc<AppState>>,
    Extension(cancel_token): Extension<CancellationToken>,
    headers: HeaderMap,
    request_id: impl AsRef<str>,
    chat_request: &ChatCompletionRequest,
) -> Result<Vec<RetrieveObject>, ServerError> {
    let mut retrieve_object_vec: Vec<RetrieveObject> = Vec::new();
    let mut set: HashSet<String> = HashSet::new();

    let mut retrieve_object = retrieve_context_with_single_qdrant_config(
        State(state.clone()),
        Extension(cancel_token.clone()),
        headers.clone(),
        request_id.as_ref(),
        chat_request,
    )
    .await?;

    if let Some(points) = retrieve_object.points.as_mut() {
        if !points.is_empty() {
            // find the duplicate points
            let mut idx_removed = vec![];
            for (idx, point) in points.iter().enumerate() {
                if set.contains(&point.source) {
                    idx_removed.push(idx);
                } else {
                    set.insert(point.source.clone());
                }
            }

            // remove the duplicate points
            if !idx_removed.is_empty() {
                let num = idx_removed.len();

                for idx in idx_removed.iter().rev() {
                    points.remove(*idx);
                }

                dual_info!(
                    "Removed {} duplicated vector search results - request_id: {}",
                    num,
                    request_id.as_ref()
                );
            }

            if !points.is_empty() {
                retrieve_object_vec.push(retrieve_object);
            }
        }
    }

    Ok(retrieve_object_vec)
}

async fn retrieve_context_with_single_qdrant_config(
    State(state): State<Arc<AppState>>,
    Extension(cancel_token): Extension<CancellationToken>,
    headers: HeaderMap,
    request_id: impl AsRef<str>,
    chat_request: &ChatCompletionRequest,
) -> Result<RetrieveObject, ServerError> {
    let request_id = request_id.as_ref();

    // get the context window from config
    let config_ctx_window = state.config.read().await.rag.context_window;

    // get context_window: chat_request.context_window prioritized CONTEXT_WINDOW
    let context_window = chat_request
        .context_window
        .or(Some(config_ctx_window))
        .unwrap_or(1);
    dual_info!(
        "Context window: {} - request_id: {}",
        context_window,
        request_id
    );

    // compute embeddings for user query
    let embedding_response = match chat_request.messages.is_empty() {
        true => {
            let err_msg = "Found empty chat messages";

            // log
            dual_error!("{} - request_id: {}", err_msg, request_id);

            return Err(ServerError::BadRequest(err_msg.to_string()));
        }
        false => {
            // get the last `n` user messages in the context window.
            // `n` is determined by the `context_window` in the chat request.
            let mut last_n_user_messages = Vec::new();
            for (idx, message) in chat_request.messages.iter().rev().enumerate() {
                if let ChatCompletionRequestMessage::User(user_message) = message {
                    if let ChatCompletionUserMessageContent::Text(text) = user_message.content() {
                        if !text.ends_with("<server-health>") {
                            last_n_user_messages.push(text.clone());
                        } else if idx == 0 {
                            let content = text.trim_end_matches("<server-health>").to_string();
                            last_n_user_messages.push(content);
                            break;
                        }
                    }
                }

                if last_n_user_messages.len() == context_window as usize {
                    break;
                }
            }

            // join the user messages in the context window into a single string
            let query_text = if !last_n_user_messages.is_empty() {
                last_n_user_messages.reverse();
                last_n_user_messages.join("\n")
            } else {
                let error_msg = "No user messages found.";

                // log
                dual_error!("{} - request_id: {}", error_msg, request_id);

                return Err(ServerError::BadRequest(error_msg.to_string()));
            };

            dual_info!(
                "Computing embeddings for user query: {} - request_id: {}",
                query_text,
                request_id
            );
            // create a embedding request
            let embedding_request = EmbeddingRequest {
                model: None,
                input: InputText::String(query_text),
                encoding_format: None,
                user: chat_request.user.clone(),
                vdb_server_url: None,
                vdb_collection_name: None,
                vdb_api_key: None,
            };

            // compute embeddings for query
            let response = crate::handlers::embeddings_handler(
                State(state.clone()),
                Extension(cancel_token.clone()),
                headers.clone(),
                Json(embedding_request),
            )
            .await?;

            // parse the response
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .map_err(|e| {
                    let err_msg = format!("Failed to parse embeddings response: {e}");

                    // log
                    dual_error!("{} - request_id: {}", err_msg, request_id);

                    ServerError::Operation(err_msg)
                })?;

            // parse the response
            serde_json::from_slice::<EmbeddingsResponse>(&bytes).map_err(|e| {
                let err_msg = format!("Failed to parse embeddings response: {e}");

                // log
                dual_error!("{} - request_id: {}", err_msg, request_id);

                ServerError::Operation(err_msg)
            })?
        }
    };

    let query_embedding: Vec<f32> = match embedding_response.data.first() {
        Some(embedding) => embedding.embedding.iter().map(|x| *x as f32).collect(),
        None => {
            let err_msg = "No embeddings returned";

            // log
            dual_error!("{} - request_id: {}", err_msg, request_id);

            return Err(ServerError::Operation(err_msg.to_string()));
        }
    };

    // perform the context retrieval
    let mut retrieve_object: RetrieveObject =
        match retrieve_context(query_embedding.as_slice(), request_id).await {
            Ok(search_result) => search_result,
            Err(e) => {
                let err_msg = format!("No point retrieved. {e}");

                // log
                dual_error!("{} - request_id: {}", err_msg, request_id);

                return Err(ServerError::Operation(err_msg));
            }
        };
    if retrieve_object.points.is_none() {
        retrieve_object.points = Some(Vec::new());
    }

    dual_debug!(
        "Got {} point(s) by vector search - request_id: {}",
        retrieve_object.points.as_ref().unwrap().len(),
        request_id
    );

    Ok(retrieve_object)
}

async fn retrieve_context(
    query_embedding: &[f32],
    request_id: impl AsRef<str>,
) -> Result<RetrieveObject, ServerError> {
    let request_id = request_id.as_ref();

    // search points from gaia-qdrant-mcp-server
    let scored_points = match MCP_VECTOR_SEARCH_CLIENT.get() {
        Some(mcp_client) => {
            // request param
            let request_param = CallToolRequestParam {
                name: "search_points".into(),
                arguments: Some(serde_json::Map::from_iter([(
                    "vector".to_string(),
                    Value::from(query_embedding.to_vec()),
                )])),
            };

            // call the search_points tool
            let tool_result = mcp_client
                .read()
                .await
                .raw
                .peer()
                .call_tool(request_param)
                .await
                .map_err(|e| {
                    let err_msg = format!("Failed to call the search_points tool: {e}");
                    dual_error!("{} - request_id: {}", err_msg, request_id);
                    ServerError::Operation(err_msg)
                })?;

            // parse the response
            let response = SearchPointsResponse::from(tool_result);

            response.result
        }
        None => {
            let err_msg = "No vector search mcp client available";
            dual_error!("{} - request_id: {}", err_msg, request_id);
            return Err(ServerError::Operation(err_msg.to_string()));
        }
    };

    dual_debug!(
        "Check and remove duplicated vector search results - request_id: {}",
        request_id
    );

    // remove duplicates, which have the same source
    let mut seen = HashSet::new();
    let unique_scored_points: Vec<ScoredPoint> = scored_points
        .into_iter()
        .filter(|point| seen.insert(point.payload.get("source").unwrap().to_string()))
        .collect();

    dual_debug!(
        "Retrieved {} unique vector search results in total - request_id: {}",
        unique_scored_points.len(),
        request_id
    );

    let ro = match unique_scored_points.is_empty() {
        true => RetrieveObject {
            points: None,
            limit: 0,
            score_threshold: 0.0,
        },
        false => {
            let mut points: Vec<RagScoredPoint> = vec![];
            for point in unique_scored_points.iter() {
                if point.payload.is_empty() {
                    continue;
                }

                dual_debug!("point: {:?}", point);

                if let Some(source) = point.payload.get("source").and_then(Value::as_str) {
                    points.push(RagScoredPoint {
                        source: source.to_string(),
                        score: point.score,
                        from: DataFrom::VectorSearch,
                    })
                }

                // For debugging purpose, log the optional search field if it exists
                if let Some(search) = point.payload.get("search").and_then(Value::as_str) {
                    dual_info!("search: {} - request_id: {}", search, request_id);
                }
            }

            RetrieveObject {
                points: Some(points),
                limit: 0,
                score_threshold: 0.0,
            }
        }
    };

    Ok(ro)
}

#[derive(Debug, Default)]
struct RagPromptBuilder;
impl MergeRagContext for RagPromptBuilder {
    fn build(
        messages: &mut Vec<endpoints::chat::ChatCompletionRequestMessage>,
        context: &[String],
        has_system_prompt: bool,
        policy: MergeRagContextPolicy,
        _rag_prompt: Option<String>,
    ) -> ChatPromptsError::Result<()> {
        if messages.is_empty() {
            dual_error!("Found empty messages in the chat request.");

            return Err(ChatPromptsError::PromptError::NoMessages);
        }

        if context.is_empty() {
            let err_msg = "No context provided.";

            // log
            dual_error!("{}", &err_msg);

            return Err(ChatPromptsError::PromptError::Operation(err_msg.into()));
        }
        let context = context[0].trim_end();

        // check rag policy
        let mut policy = policy;
        if policy == MergeRagContextPolicy::SystemMessage && !has_system_prompt {
            // log
            dual_info!("The chat model does not support system message. Switch the currect rag policy to `last-user-message`");

            policy = MergeRagContextPolicy::LastUserMessage;
        }
        match policy {
            MergeRagContextPolicy::SystemMessage => {
                match &messages[0] {
                    ChatCompletionRequestMessage::System(message) => {
                        let content = format!(
                            "You are a helpful AI assistant. Please answer the user question based on the information between **---BEGIN CONTEXT---** and **---END CONTEXT---**. Do not use any external knowledge. If the information between **---BEGIN CONTEXT---** and **---END CONTEXT---** is empty, please respond with `No relevant information found in the current knowledge base`.\n\n---BEGIN CONTEXT---\n\n{context}\n\n---END CONTEXT---",
                        );

                        let system_message = ChatCompletionRequestMessage::new_system_message(
                            content,
                            message.name().cloned(),
                        );

                        // replace the original system message
                        messages[0] = system_message;
                    }
                    _ => {
                        // compose new system message content
                        let content = format!(
                            "You are a helpful AI assistant. Please answer the user question based on the information between **---BEGIN CONTEXT---** and **---END CONTEXT---**. Do not use any external knowledge. If the information between **---BEGIN CONTEXT---** and **---END CONTEXT---** is empty, please respond with `No relevant information found in the current knowledge base`.\n\n---BEGIN CONTEXT---\n\n{context}\n\n---END CONTEXT---",
                        );

                        // create system message
                        let system_message =
                            ChatCompletionRequestMessage::new_system_message(content, None);

                        // insert system message
                        messages.insert(0, system_message);
                    }
                }

                dual_info!("Merged RAG context into system message");
            }
            MergeRagContextPolicy::LastUserMessage => {
                let len = messages.len();
                match &messages.last() {
                    Some(ChatCompletionRequestMessage::User(message)) => {
                        if let ChatCompletionUserMessageContent::Text(content) = message.content() {
                            let extened_content = format!(
                                "You are a helpful AI assistant. Please answer the user question based on the information between **---BEGIN CONTEXT---** and **---END CONTEXT---**. Do not use any external knowledge. If the information between **---BEGIN CONTEXT---** and **---END CONTEXT---** is empty, please respond with `No relevant information found in the current knowledge base`.\n\n---BEGIN CONTEXT---\n\n{context}\n\n---END CONTEXT---\n\nThe question is:\n{content}",
                            );

                            let content = ChatCompletionUserMessageContent::Text(extened_content);

                            // create user message
                            let user_message = ChatCompletionRequestMessage::new_user_message(
                                content,
                                message.name().cloned(),
                            );
                            // replace the original user message
                            messages[len - 1] = user_message;
                        }
                    }
                    _ => {
                        let err_msg =
                            "The last message in the chat request should be a user message.";

                        // log
                        dual_error!("{}", &err_msg);

                        return Err(ChatPromptsError::PromptError::BadMessages(err_msg.into()));
                    }
                }

                dual_info!("Merged RAG context into last user message");
            }
        }

        Ok(())
    }
}

// Segment the given text into chunks
pub(crate) fn chunk_text(
    text: impl AsRef<str>,
    ty: impl AsRef<str>,
    chunk_capacity: usize,
    request_id: impl AsRef<str>,
) -> Result<Vec<String>, ServerError> {
    let request_id = request_id.as_ref();

    if ty.as_ref().to_lowercase().as_str() != "txt" && ty.as_ref().to_lowercase().as_str() != "md" {
        let err_msg = "Failed to upload the target file. Only files with 'txt' and 'md' extensions are supported.";

        dual_error!("{} - request_id: {}", err_msg, request_id);

        return Err(ServerError::Operation(err_msg.into()));
    }

    match ty.as_ref().to_lowercase().as_str() {
        "txt" => {
            dual_info!("Chunk the plain text contents - request_id: {}", request_id);

            // create a text splitter
            let splitter = TextSplitter::new(chunk_capacity);

            let chunks = splitter
                .chunks(text.as_ref())
                .map(|s| s.to_string())
                .collect::<Vec<_>>();

            dual_info!("{} chunks - request_id: {}", chunks.len(), request_id);

            Ok(chunks)
        }
        "md" => {
            dual_info!("Chunk the markdown contents - request_id: {}", request_id);

            // create a markdown splitter
            let splitter = MarkdownSplitter::new(chunk_capacity);

            let chunks = splitter
                .chunks(text.as_ref())
                .map(|s| s.to_string())
                .collect::<Vec<_>>();

            dual_info!(
                "Number of chunks: {} - request_id: {}",
                chunks.len(),
                request_id
            );

            Ok(chunks)
        }
        _ => {
            let err_msg =
                "Failed to upload the target file. Only text and markdown files are supported.";

            dual_error!("{}", err_msg);

            Err(ServerError::Operation(err_msg.into()))
        }
    }
}

pub(crate) async fn qdrant_create_collection(
    vdb_server_url: impl AsRef<str>,
    vdb_api_key: impl AsRef<str>,
    collection_name: impl AsRef<str>,
    dim: usize,
    request_id: impl AsRef<str>,
) -> Result<(), ServerError> {
    let request_id = request_id.as_ref();

    dual_info!(
        "Create a collection `{}` of {} dimensions - request_id: {}",
        collection_name.as_ref(),
        dim,
        request_id
    );

    match MCP_VECTOR_SEARCH_CLIENT.get() {
        Some(mcp_client) => {
            // request param
            let request_param = match vdb_api_key.as_ref().is_empty() {
                true => CallToolRequestParam {
                    name: "create_collection".into(),
                    arguments: Some(serde_json::Map::from_iter([
                        (
                            "base_url".to_string(),
                            serde_json::Value::from(vdb_server_url.as_ref().to_string()),
                        ),
                        (
                            "name".to_string(),
                            serde_json::Value::String(collection_name.as_ref().to_string()),
                        ),
                        ("size".to_string(), serde_json::Value::from(dim as u64)),
                    ])),
                },
                false => CallToolRequestParam {
                    name: "create_collection".into(),
                    arguments: Some(serde_json::Map::from_iter([
                        (
                            "base_url".to_string(),
                            serde_json::Value::from(vdb_server_url.as_ref().to_string()),
                        ),
                        (
                            "api_key".to_string(),
                            serde_json::Value::String(vdb_api_key.as_ref().to_string()),
                        ),
                        (
                            "name".to_string(),
                            serde_json::Value::String(collection_name.as_ref().to_string()),
                        ),
                        ("size".to_string(), serde_json::Value::from(dim as u64)),
                    ])),
                },
            };

            // call the create_collection tool
            let tool_result = mcp_client
                .read()
                .await
                .raw
                .peer()
                .call_tool(request_param)
                .await
                .map_err(|e| {
                    let err_msg = format!("Failed to call the create_collection tool: {e}");
                    dual_error!("{} - request_id: {}", err_msg, request_id);
                    ServerError::Operation(err_msg)
                })?;

            // parse the response
            let response = CreateCollectionResponse::from(tool_result);

            if response.result {
                dual_info!(
                    "Collection `{}` created successfully - request_id: {}",
                    collection_name.as_ref(),
                    request_id
                );
                Ok(())
            } else {
                let err_msg = format!("Failed to create collection `{}`", collection_name.as_ref());
                dual_error!("{} - request_id: {}", err_msg, request_id);
                Err(ServerError::Operation(err_msg))
            }
        }
        None => {
            let err_msg = "No vector search mcp client found";
            dual_error!("{} - request_id: {}", err_msg, request_id);
            Err(ServerError::Operation(err_msg.to_string()))
        }
    }
}

pub(crate) async fn qdrant_persist_embeddings(
    vdb_server_url: impl AsRef<str>,
    vdb_api_key: impl AsRef<str>,
    collection_name: impl AsRef<str>,
    embeddings: &[EmbeddingObject],
    chunks: &[String],
    request_id: impl AsRef<str>,
) -> Result<(), ServerError> {
    let request_id = request_id.as_ref();

    dual_info!(
        "Persist embeddings to the Qdrant instance - request_id: {}",
        request_id
    );

    let mut points = Vec::<Point>::new();
    for embedding in embeddings {
        // convert the embedding to a vector
        let vector: Vec<_> = embedding.embedding.iter().map(|x| *x as f32).collect();

        // create a payload
        let payload = serde_json::json!({"source": chunks[embedding.index as usize]})
            .as_object()
            .map(|m| m.to_owned());

        // create a point
        let point = Point {
            id: embedding.index,
            vector,
            payload: payload.unwrap_or_default(),
        };

        points.push(point);
    }

    dual_info!(
        "{} points to be upserted - request_id: {}",
        points.len(),
        request_id
    );

    match MCP_VECTOR_SEARCH_CLIENT.get() {
        Some(mcp_client) => {
            // request param
            let request_param = match vdb_api_key.as_ref().is_empty() {
                true => CallToolRequestParam {
                    name: "upsert_points".into(),
                    arguments: Some(serde_json::Map::from_iter([
                        (
                            "base_url".to_string(),
                            serde_json::Value::from(vdb_server_url.as_ref().to_string()),
                        ),
                        (
                            "name".to_string(),
                            serde_json::Value::from(collection_name.as_ref().to_string()),
                        ),
                        (
                            "points".to_string(),
                            serde_json::Value::Array(
                                points
                                    .into_iter()
                                    .map(|p| serde_json::to_value(p).unwrap())
                                    .collect(),
                            ),
                        ),
                    ])),
                },
                false => CallToolRequestParam {
                    name: "upsert_points".into(),
                    arguments: Some(serde_json::Map::from_iter([
                        (
                            "base_url".to_string(),
                            serde_json::Value::from(vdb_server_url.as_ref().to_string()),
                        ),
                        (
                            "api_key".to_string(),
                            serde_json::Value::String(vdb_api_key.as_ref().to_string()),
                        ),
                        (
                            "name".to_string(),
                            serde_json::Value::from(collection_name.as_ref().to_string()),
                        ),
                        (
                            "points".to_string(),
                            serde_json::Value::Array(
                                points
                                    .into_iter()
                                    .map(|p| serde_json::to_value(p).unwrap())
                                    .collect(),
                            ),
                        ),
                    ])),
                },
            };

            // call the upsert_points tool
            let tool_result = mcp_client
                .read()
                .await
                .raw
                .peer()
                .call_tool(request_param)
                .await
                .map_err(|e| {
                    let err_msg = format!("Failed to call the upsert_points tool: {e}");
                    dual_error!("{} - request_id: {}", err_msg, request_id);
                    ServerError::Operation(err_msg)
                })?;
            let response = UpsertPointsResponse::from(tool_result);

            dual_info!(
                "Upsert points - Status: {} - Time: {} - request_id: {}",
                response.status,
                response.time,
                request_id
            );

            Ok(())
        }
        None => {
            let err_msg = "No vector search mcp client found";
            dual_error!("{} - request_id: {}", err_msg, request_id);
            Err(ServerError::Operation(err_msg.to_string()))
        }
    }
}

fn calculate_hash(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Normalize scores with min-max normalization
fn min_max_normalize(scores: &HashMap<u64, f64>) -> HashMap<u64, f64> {
    if scores.is_empty() {
        return scores.clone();
    }

    let min_score = scores.values().cloned().fold(f64::INFINITY, f64::min);
    let max_score = scores.values().cloned().fold(f64::NEG_INFINITY, f64::max);

    dual_debug!(
        "Normalize scores: min_score: {}, max_score: {}",
        min_score,
        max_score
    );

    // Add a small offset to ensure scores are in (0,1)
    const EPSILON: f64 = 1e-6;
    let range = max_score - min_score;
    let offset = if range > 0.0 { EPSILON } else { 0.0 };

    scores
        .iter()
        .map(|(&doc_id, &score)| {
            let normalized_score = if range > 0.0 {
                // Map to (0,1) by adding offset and scaling
                offset + (1.0 - 2.0 * offset) * (score - min_score) / range
            } else {
                0.5 // If all scores are the same, map to middle of interval
            };

            dual_debug!(
                "Normalize score: doc_id: {}, score: {}, normalized_score: {}",
                doc_id,
                score,
                normalized_score
            );

            (doc_id, normalized_score)
        })
        .collect()
}

/// Normalize scores with z-score normalization and map to [0,1] using sigmoid
fn _z_score_normalize(scores: &HashMap<u64, f64>) -> HashMap<u64, f64> {
    if scores.is_empty() {
        return scores.clone();
    }

    // Calculate mean
    let mean = scores.values().sum::<f64>() / scores.len() as f64;

    // Calculate standard deviation
    let variance = scores
        .values()
        .map(|&score| {
            let diff = score - mean;
            diff * diff
        })
        .sum::<f64>()
        / scores.len() as f64;
    let std_dev = variance.sqrt();

    dual_debug!("Z-score normalize: mean: {}, std_dev: {}", mean, std_dev);

    scores
        .iter()
        .map(|(&doc_id, &score)| {
            let z_score = if std_dev > 0.0 {
                (score - mean) / std_dev
            } else {
                0.0
            };

            // Apply sigmoid function to map z-score to [0,1]
            // sigmoid(x) = 1 / (1 + e^(-x))
            let normalized_score = 1.0 / (1.0 + (-z_score).exp());

            dual_debug!(
                "Z-score normalize: doc_id: {}, score: {}, z_score: {}, normalized_score: {}",
                doc_id,
                score,
                z_score,
                normalized_score
            );

            (doc_id, normalized_score)
        })
        .collect()
}

/// Fuse keyword search and vector search scores with min-max normalization and weighted fusion
fn weighted_fusion(
    kw_search_scores: HashMap<u64, f64>,
    vector_search_scores: HashMap<u64, f64>,
    alpha: f64,
) -> HashMap<u64, f64> {
    match (kw_search_scores.is_empty(), vector_search_scores.is_empty()) {
        (false, false) => {
            dual_debug!("Fusing keyword and vector search results");

            // Normalize keyword search scores
            let kw_normalized = min_max_normalize(&kw_search_scores);
            // Normalize vector search scores
            let vector_normalized = min_max_normalize(&vector_search_scores);

            // filter out duplicates
            let all_doc_ids: HashSet<u64> = kw_search_scores
                .keys()
                .chain(vector_search_scores.keys())
                .cloned()
                .collect();

            // Calculate fusion scores
            all_doc_ids
                .into_iter()
                .map(|doc_id| {
                    let k_score = *kw_normalized.get(&doc_id).unwrap_or(&0.0);
                    let v_score = *vector_normalized.get(&doc_id).unwrap_or(&0.0);

                    if k_score > 0.0 && v_score > 0.0 {
                        let fused_score = alpha * k_score + (1.0 - alpha) * v_score;

                        dual_debug!(
                            "Fusing scores: doc_id: {}, k_score: {}, v_score: {}, fused_score: {}",
                            doc_id,
                            k_score,
                            v_score,
                            fused_score
                        );

                        (doc_id, fused_score)
                    } else if k_score > 0.0 {
                        dual_debug!(
                            "Fusing scores: doc_id: {}, k_score: {}, v_score: {}",
                            doc_id,
                            k_score,
                            v_score,
                        );

                        (doc_id, k_score)
                    } else {
                        dual_debug!(
                            "Fusing scores: doc_id: {}, k_score: {}, v_score: {}",
                            doc_id,
                            k_score,
                            v_score,
                        );

                        (doc_id, v_score)
                    }
                })
                .collect()
        }
        (false, true) => {
            dual_debug!("Only keyword search results are available in the fusion");

            // Normalize keyword search scores
            min_max_normalize(&kw_search_scores)
        }
        (true, false) => {
            dual_debug!("Only vector search results are available in the fusion");

            // Normalize vector search scores
            min_max_normalize(&vector_search_scores)
        }
        (true, true) => {
            dual_warn!("Both keyword search and vector search scores are empty in the fusion");
            // Return empty HashMap
            HashMap::new()
        }
    }
}

async fn _extract_keywords_by_llm(
    State(state): State<Arc<AppState>>,
    text: impl AsRef<str>,
    request_id: impl AsRef<str>,
) -> ServerResult<String> {
    let request_id = request_id.as_ref();
    let text = text.as_ref();
    // let user_prompt  = format!(
    //     "Extract the keywords from the following text. Avoid stop words, filler words, or overly generic terms (e.g., “how”, “can”, “thing”, “way”). The keywords should be separated by spaces.\n\nText: {text:#?}",
    // );
    let user_prompt  = format!(
        "You are a multilingual keyword extractor. Your task is to extract the most relevant and concise keywords or key phrases from the given user query. The keywords should satisfying the following requirements:\n- Detect the language of the query automatically.\n- Return 3 to 7 keywords or keyphrases that best represent the query's core intent.\n- Keep the extracted keywords in the **original language** (do not translate).\n- Include **multi-word expressions** if they convey meaningful concepts.\n- The keywords should be separated by spaces.\n- Avoid stop words, filler words, or overly generic terms.\n\n### Input Query\n{text:#?}",
    );

    let user_message = ChatCompletionRequestMessage::new_user_message(
        ChatCompletionUserMessageContent::Text(user_prompt),
        None,
    );

    // create a request
    let request = ChatCompletionRequestBuilder::new(&[user_message]).build();

    info!(
        "request for getting keywords:\n{}",
        serde_json::to_string_pretty(&request).unwrap()
    );

    // get the chat server
    let target_server_info = {
        let servers = state.server_group.read().await;
        let chat_servers = match servers.get(&ServerKind::chat) {
            Some(servers) => servers,
            None => {
                let err_msg = "No chat server available";
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg.to_string()));
            }
        };

        match chat_servers.next().await {
            Ok(target_server_info) => target_server_info,
            Err(e) => {
                let err_msg = format!("Failed to get the chat server: {e}");
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg));
            }
        }
    };

    let chat_service_url = format!(
        "{}/v1/chat/completions",
        target_server_info.url.trim_end_matches('/')
    );
    dual_info!(
        "Forward the chat request to {} - request_id: {}",
        chat_service_url,
        request_id
    );

    // Create a request client that can be cancelled
    let response = reqwest::Client::new()
        .post(&chat_service_url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| {
            let err_msg = format!("Failed to send the chat request: {e}");
            dual_error!("{} - request_id: {}", err_msg, request_id);
            ServerError::Operation(err_msg)
        })?;

    let chat_completion_object = response.json::<ChatCompletionObject>().await.map_err(|e| {
        let err_msg = format!("Failed to parse the chat response: {e}");
        dual_error!("{} - request_id: {}", err_msg, request_id);
        ServerError::Operation(err_msg)
    })?;

    let content = chat_completion_object.choices[0]
        .message
        .content
        .as_ref()
        .unwrap();

    Ok(content.to_string())
}

async fn _call_keyword_search_mcp_server_new_old(
    tool_calls: &[ToolCall],
    request: &mut ChatCompletionRequest,
    request_id: impl AsRef<str>,
) -> ServerResult<Vec<KwSearchHit>> {
    let request_id = request_id.as_ref();

    // get the user id from the request
    let user_id = match request.user.as_ref() {
        Some(user_id) => user_id,
        None => {
            let err_msg = "User ID is not found in the request";
            dual_error!("{} - request_id: {}", err_msg, request_id);
            return Err(ServerError::Operation(err_msg.to_string()));
        }
    };

    // let tool_calls = assistant_message.tool_calls.clone();
    let tool_call = &tool_calls[0];
    let tool_name = tool_call.function.name.as_str();
    let tool_args = &tool_call.function.arguments;

    dual_debug!(
        "tool name: {}, tool args: {} - request_id: {}",
        tool_name,
        tool_args,
        request_id
    );

    // convert the func_args to a json object
    let arguments =
        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(tool_args).ok();

    // get the mcp client by user id and tool name, then call the tool
    let mut kw_hits: Vec<KwSearchHit> = Vec::new();
    match USER_TO_MCP_TOOLS.get() {
        Some(user_to_mcp_tools) => {
            let user_to_mcp_tools = user_to_mcp_tools.read().await;
            match user_to_mcp_tools.get(user_id) {
                Some(mcp_tools) => {
                    let mcp_tools = mcp_tools.read().await;
                    match mcp_tools.get(tool_name) {
                        Some(mcp_client_name) => {
                            match USER_TO_MCP_CLIENTS.get() {
                                Some(user_to_mcp_clients) => {
                                    let user_to_mcp_clients = user_to_mcp_clients.read().await;
                                    match user_to_mcp_clients.get(user_id) {
                                        Some(mcp_clients) => {
                                            let mcp_clients = mcp_clients.read().await;
                                            match mcp_clients.get(mcp_client_name) {
                                                Some(mcp_client) => {
                                                    match mcp_client.read().await.raw.peer_info() {
                                                        Some(peer_info) => {
                                                            match peer_info
                                                                .server_info
                                                                .name
                                                                .as_str()
                                                            {
                                                                "gaia-kwsearch-mcp-server" => {
                                                                    // call a tool
                                                                    let request_param =
                                                                        CallToolRequestParam {
                                                                            name: tool_name
                                                                                .to_string()
                                                                                .into(),
                                                                            arguments,
                                                                        };
                                                                    let mcp_tool_result = mcp_client
                                                                        .read()
                                                                        .await
                                                                        .raw
                                                                        .peer()
                                                                        .call_tool(request_param)
                                                                        .await
                                                                        .map_err(|e| {
                                                                            dual_error!(
                                                                                "Failed to call the tool: {}",
                                                                                e
                                                                            );
                                                                            ServerError::Operation(e.to_string())
                                                                        })?;

                                                                    dual_debug!(
                                                                        "{} - request_id: {}",
                                                                        serde_json::to_string_pretty(&mcp_tool_result).unwrap(),
                                                                        request_id
                                                                    );

                                                                    let search_response = SearchDocumentsResponse::from(mcp_tool_result.clone());
                                                                    kw_hits = search_response.hits;

                                                                    let kw_hits_str = serde_json::to_string_pretty(&kw_hits).unwrap();
                                                                    dual_debug!("kw_hits: {} - request_id: {}", kw_hits_str, request_id);
                                                                }
                                                                "gaia-tidb-mcp-server" => {
                                                                    // call a tool
                                                                    let request_param =
                                                                        CallToolRequestParam {
                                                                            name: tool_name
                                                                                .to_string()
                                                                                .into(),
                                                                            arguments,
                                                                        };
                                                                    let mcp_tool_result = mcp_client
                                                                        .read()
                                                                        .await
                                                                        .raw
                                                                        .peer()
                                                                        .call_tool(request_param)
                                                                        .await
                                                                        .map_err(|e| {
                                                                            dual_error!(
                                                                                "Failed to call the tool: {}",
                                                                                e
                                                                            );
                                                                            ServerError::Operation(e.to_string())
                                                                        })?;

                                                                    dual_debug!(
                                                                        "{} - request_id: {}",
                                                                        serde_json::to_string_pretty(&mcp_tool_result).unwrap(),
                                                                        request_id
                                                                    );

                                                                    // parse tool result
                                                                    let search_response =
                                                                        TidbSearchResponse::from(
                                                                            mcp_tool_result,
                                                                        );

                                                                    if !search_response
                                                                        .hits
                                                                        .is_empty()
                                                                    {
                                                                        for hit in search_response
                                                                            .hits
                                                                            .iter()
                                                                        {
                                                                            let kw_hit =
                                                                                KwSearchHit {
                                                                                    title: hit
                                                                                        .title
                                                                                        .clone(),
                                                                                    content: hit
                                                                                        .content
                                                                                        .clone(),
                                                                                    score: 0.0,
                                                                                };

                                                                            kw_hits.push(kw_hit);
                                                                        }
                                                                    }
                                                                }
                                                                "gaia-elastic-mcp-server" => {
                                                                    // request param
                                                                    let request_param =
                                                                        CallToolRequestParam {
                                                                            name: tool_name
                                                                                .to_string()
                                                                                .into(),
                                                                            arguments,
                                                                        };

                                                                    // call tool
                                                                    let mcp_tool_result = mcp_client
                                                                        .read()
                                                                        .await
                                                                        .raw
                                                                        .peer()
                                                                        .call_tool(request_param)
                                                                        .await
                                                                        .map_err(|e| {
                                                                            let err_msg = format!("Failed to call the tool: {e}");
                                                                            dual_error!("{} - request_id: {}", err_msg, request_id);
                                                                            ServerError::Operation(err_msg)
                                                                        })?;

                                                                    // parse tool result
                                                                    let search_response =
                                                                        SearchResponse::from(
                                                                            mcp_tool_result,
                                                                        );

                                                                    if !search_response
                                                                        .hits
                                                                        .hits
                                                                        .is_empty()
                                                                    {
                                                                        for hit in search_response
                                                                            .hits
                                                                            .hits
                                                                            .iter()
                                                                        {
                                                                            let score = hit.score;
                                                                            let title = hit
                                                                                .source
                                                                                .get("title")
                                                                                .unwrap()
                                                                                .as_str()
                                                                                .unwrap()
                                                                                .to_string();
                                                                            let content = hit
                                                                                .source
                                                                                .get("content")
                                                                                .unwrap()
                                                                                .as_str()
                                                                                .unwrap()
                                                                                .to_string();

                                                                            let kw_hit =
                                                                                KwSearchHit {
                                                                                    title,
                                                                                    content,
                                                                                    score,
                                                                                };

                                                                            kw_hits.push(kw_hit);
                                                                        }
                                                                    }
                                                                }
                                                                _ => {
                                                                    let err_msg = format!(
                                                                        "Unsupported MCP server: {}",
                                                                        &peer_info.server_info.name
                                                                    );
                                                                    dual_error!(
                                                                        "{} - request_id: {}",
                                                                        &err_msg,
                                                                        request_id
                                                                    );
                                                                    return Err(
                                                                        ServerError::Operation(
                                                                            err_msg,
                                                                        ),
                                                                    );
                                                                }
                                                            }
                                                        }
                                                        None => {
                                                            let err_msg = "Failed to get the server info from MCP server";
                                                            dual_error!(
                                                                "{} - request_id: {}",
                                                                err_msg,
                                                                request_id
                                                            );
                                                            return Err(ServerError::Operation(
                                                                err_msg.to_string(),
                                                            ));
                                                        }
                                                    }
                                                }
                                                None => {
                                                    let err_msg = format!(
                                                        "Not found the MCP client name `{mcp_client_name}` in USER_TO_MCP_CLIENTS"
                                                    );
                                                    dual_error!(
                                                        "{} - request_id: {}",
                                                        err_msg,
                                                        request_id
                                                    );
                                                    return Err(ServerError::Operation(
                                                        err_msg.to_string(),
                                                    ));
                                                }
                                            }
                                        }
                                        None => {
                                            let err_msg =
                                                "Not found the user ID in USER_TO_MCP_CLIENTS";
                                            dual_error!("{} - request_id: {}", err_msg, request_id);
                                            return Err(ServerError::Operation(
                                                err_msg.to_string(),
                                            ));
                                        }
                                    }
                                }
                                None => {
                                    let err_msg = "USER_TO_MCP_CLIENTS is empty or not initialized";
                                    dual_error!("{} - request_id: {}", err_msg, request_id);
                                    return Err(ServerError::Operation(err_msg.to_string()));
                                }
                            }
                        }
                        None => {
                            dual_error!(
                                "Failed to find the MCP client with tool name: {} - request_id: {}",
                                tool_name,
                                request_id,
                            );
                            return Err(ServerError::McpNotFoundClient);
                        }
                    }
                }
                None => {
                    let err_msg = "Not found the user ID in USER_TO_MCP_TOOLS";
                    dual_error!("{} - request_id: {}", err_msg, request_id);
                    return Err(ServerError::Operation(err_msg.to_string()));
                }
            }
        }
        None => {
            let err_msg = "USER_TO_MCP_TOOLS is empty or not initialized";
            dual_error!("{} - request_id: {}", err_msg, request_id);
            return Err(ServerError::Operation(err_msg.to_string()));
        }
    };

    // erase mcp tools from USER_TO_MCP_TOOLS by user id
    if let Some(user_to_mcp_tools) = USER_TO_MCP_TOOLS.get() {
        let mut user_to_mcp_tools = user_to_mcp_tools.write().await;
        user_to_mcp_tools.remove(user_id);

        dual_debug!(
            "Erase mcp tools from USER_TO_MCP_TOOLS by user id: {} - request_id: {}",
            user_id,
            request_id
        );
    }

    // erase mcp clients from USER_TO_MCP_CLIENTS by user id
    if let Some(user_to_mcp_clients) = USER_TO_MCP_CLIENTS.get() {
        let mut user_to_mcp_clients = user_to_mcp_clients.write().await;
        user_to_mcp_clients.remove(user_id);

        dual_debug!(
            "Erase mcp clients from USER_TO_MCP_CLIENTS by user id: {} - request_id: {}",
            user_id,
            request_id
        );
    }

    Ok(kw_hits)
}

async fn call_keyword_search_mcp_server_new(
    tool_calls: &[ToolCall],
    request_id: impl AsRef<str>,
) -> ServerResult<Vec<KwSearchHit>> {
    let request_id = request_id.as_ref();

    // get the tool call from the tool calls
    let tool_call = &tool_calls[0];
    let tool_name = tool_call.function.name.as_str();
    let tool_args = &tool_call.function.arguments;

    dual_debug!(
        "tool name: {}, tool args: {} - request_id: {}",
        tool_name,
        tool_args,
        request_id
    );

    // convert the func_args to a json object
    let arguments =
        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(tool_args).ok();

    // call the `search` tool of the keyword search mcp server
    let mut kw_hits: Vec<KwSearchHit> = Vec::new();
    match MCP_KEYWORD_SEARCH_CLIENT.get() {
        Some(mcp_client) => match mcp_client.read().await.raw.peer_info() {
            Some(peer_info) => match peer_info.server_info.name.as_str() {
                "gaia-kwsearch-mcp-server" => {
                    // call a tool
                    let request_param = CallToolRequestParam {
                        name: tool_name.to_string().into(),
                        arguments,
                    };
                    let mcp_tool_result = mcp_client
                        .read()
                        .await
                        .raw
                        .peer()
                        .call_tool(request_param)
                        .await
                        .map_err(|e| {
                            dual_error!("Failed to call the tool: {}", e);
                            ServerError::Operation(e.to_string())
                        })?;

                    dual_debug!(
                        "{} - request_id: {}",
                        serde_json::to_string_pretty(&mcp_tool_result).unwrap(),
                        request_id
                    );

                    let search_response = SearchDocumentsResponse::from(mcp_tool_result.clone());
                    kw_hits = search_response.hits;

                    let kw_hits_str = serde_json::to_string_pretty(&kw_hits).unwrap();
                    dual_debug!("kw_hits: {} - request_id: {}", kw_hits_str, request_id);
                }
                "gaia-tidb-mcp-server" => {
                    // call a tool
                    let request_param = CallToolRequestParam {
                        name: tool_name.to_string().into(),
                        arguments,
                    };
                    let mcp_tool_result = mcp_client
                        .read()
                        .await
                        .raw
                        .peer()
                        .call_tool(request_param)
                        .await
                        .map_err(|e| {
                            dual_error!("Failed to call the tool: {}", e);
                            ServerError::Operation(e.to_string())
                        })?;

                    dual_debug!(
                        "{} - request_id: {}",
                        serde_json::to_string_pretty(&mcp_tool_result).unwrap(),
                        request_id
                    );

                    // parse tool result
                    let search_response = TidbSearchResponse::from(mcp_tool_result);

                    if !search_response.hits.is_empty() {
                        for hit in search_response.hits.iter() {
                            let kw_hit = KwSearchHit {
                                title: hit.title.clone(),
                                content: hit.content.clone(),
                                score: 0.0,
                            };

                            kw_hits.push(kw_hit);
                        }
                    }
                }
                "gaia-elastic-mcp-server" => {
                    // request param
                    let request_param = CallToolRequestParam {
                        name: tool_name.to_string().into(),
                        arguments,
                    };

                    // call tool
                    let mcp_tool_result = mcp_client
                        .read()
                        .await
                        .raw
                        .peer()
                        .call_tool(request_param)
                        .await
                        .map_err(|e| {
                            let err_msg = format!("Failed to call the tool: {e}");
                            dual_error!("{} - request_id: {}", err_msg, request_id);
                            ServerError::Operation(err_msg)
                        })?;

                    // parse tool result
                    let search_response = SearchResponse::from(mcp_tool_result);

                    if !search_response.hits.hits.is_empty() {
                        for hit in search_response.hits.hits.iter() {
                            let score = hit.score;
                            let title = hit
                                .source
                                .get("title")
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_string();
                            let content = hit
                                .source
                                .get("content")
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_string();

                            let kw_hit = KwSearchHit {
                                title,
                                content,
                                score,
                            };

                            kw_hits.push(kw_hit);
                        }
                    }
                }
                _ => {
                    let err_msg =
                        format!("Unsupported MCP server: {}", &peer_info.server_info.name);
                    dual_error!("{} - request_id: {}", &err_msg, request_id);
                    return Err(ServerError::Operation(err_msg));
                }
            },
            None => {
                let err_msg = "Failed to get the server info from MCP server";
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg.to_string()));
            }
        },
        None => {
            let warn_msg = "No keyword search mcp server connected";
            dual_warn!("{} - request_id: {}", warn_msg, request_id);
        }
    }

    Ok(kw_hits)
}
