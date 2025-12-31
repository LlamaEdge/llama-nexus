---
name: weather-query
description: Query weather information for cities. Use when users ask about weather, temperature, rain, or if they need an umbrella.
license: MIT
compatibility: Requires weather MCP server
allowed-tools: weather forecast
metadata:
  author: test
  version: "1.0"
---

# Weather Query Skill

When users ask about weather, follow these steps:

## Workflow

1. **Identify City**: Extract city name from user query
   - If city is not specified, ask the user
   - Support both Chinese and English city names

2. **Call Weather Tool**: Use MCP tools to query weather
   - `weather`: Query current weather
   - `forecast`: Query weather forecast

3. **Format Output**: Present weather information in a friendly format

## Output Format

```
## [City] Weather

**Current Weather**
- Condition: [sunny/cloudy/rainy etc]
- Temperature: [temp]°C
- Humidity: [humidity]%

**Suggestion**
- [Suggestions based on weather]
```

## Notes

- If MCP tool call fails, inform user and suggest retry
- Default temperature unit is Celsius
