# test /v1/audio/transcriptions endpoint
POST http://localhost:3389/v1/audio/transcriptions
[MultipartFormData]
file: file,test.wav;
language: en
max_context: 100
HTTP 200
[Asserts]
jsonpath "$.text" contains "This is a test record"