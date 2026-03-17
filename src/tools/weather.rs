//! Weather tool — fetch current weather and forecasts.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use super::{Tool, ToolResult};

pub struct WeatherTool;
impl WeatherTool { pub fn new() -> Self { Self } }

#[async_trait]
impl Tool for WeatherTool {
    fn name(&self) -> &str { "weather" }
    fn description(&self) -> &str {
        "Fetch weather data. Operations: current (get current weather), forecast (get multi-day forecast)."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": { "type": "string", "description": "One of: current, forecast" },
                "city": { "type": "string", "description": "City name (e.g. Taipei, Tokyo, London)" },
                "days": { "type": "integer", "description": "Forecast days (default 3, max 7)" }
            },
            "required": ["operation", "city"]
        })
    }
    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let operation = args["operation"].as_str().unwrap_or("").trim();
        let city = args["city"].as_str().unwrap_or("").trim();
        let days = args["days"].as_u64().unwrap_or(3).min(7);

        if city.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing required parameter: city".into() });
        }

        match operation {
            "current" => {
                let url = format!("https://wttr.in/{}?format=j1", urlencoding::encode(city));
                match reqwest::get(&url).await {
                    Ok(resp) => {
                        if !resp.status().is_success() {
                            return Ok(ToolResult { success: false, output: format!("Weather API error: {}", resp.status()) });
                        }
                        match resp.json::<Value>().await {
                            Ok(data) => {
                                let current = &data["current_condition"][0];
                                let temp_c = current["temp_C"].as_str().unwrap_or("?");
                                let feels = current["FeelsLikeC"].as_str().unwrap_or("?");
                                let desc = current["weatherDesc"][0]["value"].as_str().unwrap_or("?");
                                let humidity = current["humidity"].as_str().unwrap_or("?");
                                let wind = current["windspeedKmph"].as_str().unwrap_or("?");
                                let output = format!(
                                    "Weather in {}:\nTemperature: {}°C (feels like {}°C)\nCondition: {}\nHumidity: {}%\nWind: {} km/h",
                                    city, temp_c, feels, desc, humidity, wind
                                );
                                Ok(ToolResult { success: true, output })
                            }
                            Err(e) => Ok(ToolResult { success: false, output: format!("Failed to parse weather data: {}", e) })
                        }
                    }
                    Err(e) => Ok(ToolResult { success: false, output: format!("Failed to fetch weather: {}", e) })
                }
            }
            "forecast" => {
                let url = format!("https://wttr.in/{}?format=j1", urlencoding::encode(city));
                match reqwest::get(&url).await {
                    Ok(resp) => {
                        if !resp.status().is_success() {
                            return Ok(ToolResult { success: false, output: format!("Weather API error: {}", resp.status()) });
                        }
                        match resp.json::<Value>().await {
                            Ok(data) => {
                                let mut lines = vec![format!("{}-day forecast for {}:", days, city)];
                                if let Some(weather) = data["weather"].as_array() {
                                    for (i, day) in weather.iter().take(days as usize).enumerate() {
                                        let date = day["date"].as_str().unwrap_or("?");
                                        let max = day["maxtempC"].as_str().unwrap_or("?");
                                        let min = day["mintempC"].as_str().unwrap_or("?");
                                        let desc = day["hourly"][4]["weatherDesc"][0]["value"].as_str().unwrap_or("?");
                                        lines.push(format!("Day {}: {} — {}°C to {}°C, {}", i+1, date, min, max, desc));
                                    }
                                }
                                Ok(ToolResult { success: true, output: lines.join("\n") })
                            }
                            Err(e) => Ok(ToolResult { success: false, output: format!("Failed to parse forecast: {}", e) })
                        }
                    }
                    Err(e) => Ok(ToolResult { success: false, output: format!("Failed to fetch forecast: {}", e) })
                }
            }
            _ => Ok(ToolResult { success: false, output: format!("Unknown operation: {}. Use: current, forecast", operation) })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_weather_name() { assert_eq!(WeatherTool::new().name(), "weather"); }

    #[test]
    fn test_weather_schema() {
        let schema = WeatherTool::new().parameters_schema();
        assert!(schema["properties"]["city"].is_object());
        assert!(schema["properties"]["operation"].is_object());
    }

    #[tokio::test]
    async fn test_weather_missing_city() {
        let result = WeatherTool::new().execute(json!({"operation": "current", "city": ""})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_weather_unknown_operation() {
        let result = WeatherTool::new().execute(json!({"operation": "xyz", "city": "Taipei"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown operation"));
    }

    #[test]
    fn test_weather_description() {
        let tool = WeatherTool::new(); let desc = tool.description();
        assert!(desc.contains("weather"));
    }

    #[tokio::test]
    async fn test_weather_missing_operation() {
        let result = WeatherTool::new().execute(json!({"city": "Tokyo"})).await.unwrap();
        assert!(!result.success);
    }
}
