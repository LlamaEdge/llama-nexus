# test /v1/audio/translations endpoint
POST http://localhost:3389/v1/audio/translations
[MultipartFormData]
file: file,test_cn.wav;
language: zh
HTTP 200
[Asserts]
jsonpath "$.text" contains "This is a Chinese"