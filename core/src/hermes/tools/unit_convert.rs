//! `hermes_unit_convert` — common physical unit conversions.
//!
//! Supports length (m/km/mi/ft), mass (g/kg/lb/oz), temperature (c/f/k).

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{HermesTool, ToolError, ToolResult};

pub struct UnitConvert;

#[async_trait]
impl HermesTool for UnitConvert {
    fn name(&self) -> &'static str {
        "hermes_unit_convert"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "hermes_unit_convert",
                "description": "Convert `value` from `from` unit to `to` unit. \
                    Supported units: length (m/km/mi/ft), mass (g/kg/lb/oz), temperature (c/f/k).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "value": {"type": "number"},
                        "from":  {"type": "string"},
                        "to":    {"type": "string"}
                    },
                    "required": ["value", "from", "to"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let value = args
            .get("value")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| ToolError::BadArgs("value required (number)".into()))?;
        let from = args
            .get("from")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("from required".into()))?
            .to_lowercase();
        let to = args
            .get("to")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("to required".into()))?
            .to_lowercase();
        let result = convert(value, &from, &to).map_err(ToolError::Invalid)?;
        Ok(json!({ "result": result, "unit": to }))
    }
}

pub(crate) fn convert(value: f64, from: &str, to: &str) -> Result<f64, String> {
    // Length — base = metres.
    let length: &[(&str, f64)] = &[("m", 1.0), ("km", 1000.0), ("mi", 1609.344), ("ft", 0.3048)];
    // Mass — base = grams.
    let mass: &[(&str, f64)] = &[
        ("g", 1.0),
        ("kg", 1000.0),
        ("lb", 453.59237),
        ("oz", 28.349523125),
    ];
    // Temperature — handled separately because it's affine, not linear.
    if matches!(from, "c" | "f" | "k") || matches!(to, "c" | "f" | "k") {
        return temp(value, from, to);
    }
    if let (Some(a), Some(b)) = (
        length.iter().find(|(u, _)| *u == from),
        length.iter().find(|(u, _)| *u == to),
    ) {
        return Ok(value * a.1 / b.1);
    }
    if let (Some(a), Some(b)) = (
        mass.iter().find(|(u, _)| *u == from),
        mass.iter().find(|(u, _)| *u == to),
    ) {
        return Ok(value * a.1 / b.1);
    }
    Err(format!("unsupported unit pair: {} → {}", from, to))
}

fn temp(value: f64, from: &str, to: &str) -> Result<f64, String> {
    // Convert to Kelvin first.
    let kelvin = match from {
        "k" => value,
        "c" => value + 273.15,
        "f" => (value - 32.0) * 5.0 / 9.0 + 273.15,
        _ => return Err(format!("temp: unknown from {}", from)),
    };
    Ok(match to {
        "k" => kelvin,
        "c" => kelvin - 273.15,
        "f" => (kelvin - 273.15) * 9.0 / 5.0 + 32.0,
        _ => return Err(format!("temp: unknown to {}", to)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn km_to_mi_is_correct() {
        let tool = UnitConvert;
        let r = tool
            .call(&json!({"value": 1.0, "from": "km", "to": "mi"}))
            .await
            .unwrap();
        let v = r["result"].as_f64().unwrap();
        assert!((v - 0.6213711922).abs() < 1e-6, "got {}", v);
    }

    #[tokio::test]
    async fn celsius_to_fahrenheit_is_correct() {
        let tool = UnitConvert;
        let r = tool
            .call(&json!({"value": 100.0, "from": "c", "to": "f"}))
            .await
            .unwrap();
        assert_eq!(r["result"], 212.0);
    }

    #[tokio::test]
    async fn unsupported_unit_pair_is_invalid() {
        let tool = UnitConvert;
        let err = tool
            .call(&json!({"value": 1.0, "from": "km", "to": "lb"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Invalid(_)));
    }
}
