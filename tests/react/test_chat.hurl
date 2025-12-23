# test /v1/chat/completions endpoint
POST http://localhost:3389/v1/chat/completions
Accept: application/json
Content-Type: application/json
```json
{
    "messages": [
        {
            "role": "user",
            "content": "32 + 23 = ?"
        }
    ],
    "model": "Qwen3-4B",
    "stream": false
}
```
HTTP 200
[Asserts]
jsonpath "$.model" == "Qwen3-4B"
jsonpath "$.choices[0].message.content" contains "55"
