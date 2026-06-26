//! `skill_color_hex_rgb` — convert between #RRGGBB and rgb(R,G,B).
//!
//! Two ops:
//!   * `to_rgb`: input `#RRGGBB` (with or without `#`, 3- or 6-digit) →
//!     `{ "rgb": "rgb(R, G, B)", "r": R, "g": G, "b": B }`.
//!   * `to_hex`: input `rgb(R, G, B)` or `{r,g,b}` keys →
//!     `{ "hex": "#RRGGBB" }`.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{SkillTool, ToolError, ToolResult};

pub struct ColorHexRgb;

#[async_trait]
impl SkillTool for ColorHexRgb {
    fn name(&self) -> &'static str {
        "skill_color_hex_rgb"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "skill_color_hex_rgb",
                "description": "Convert color between #RRGGBB hex and rgb(R, G, B) form. \
                    `op=to_rgb` takes `hex` (3- or 6-digit, leading # optional). \
                    `op=to_hex` takes `rgb` string like 'rgb(255, 64, 0)'.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "op":  { "type": "string", "enum": ["to_rgb", "to_hex"] },
                        "hex": { "type": "string" },
                        "rgb": { "type": "string" }
                    },
                    "required": ["op"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let op = args
            .get("op")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("op required".into()))?;
        match op {
            "to_rgb" => {
                let hex = args
                    .get("hex")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::BadArgs("hex required for to_rgb".into()))?;
                let (r, g, b) = parse_hex(hex).map_err(ToolError::Invalid)?;
                Ok(json!({ "rgb": format!("rgb({}, {}, {})", r, g, b), "r": r, "g": g, "b": b }))
            }
            "to_hex" => {
                let rgb = args
                    .get("rgb")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::BadArgs("rgb required for to_hex".into()))?;
                let (r, g, b) = parse_rgb(rgb).map_err(ToolError::Invalid)?;
                Ok(json!({ "hex": format!("#{:02X}{:02X}{:02X}", r, g, b) }))
            }
            other => Err(ToolError::BadArgs(format!("unknown op: {}", other))),
        }
    }
}

fn parse_hex(raw: &str) -> Result<(u8, u8, u8), String> {
    let s = raw.trim().trim_start_matches('#');
    let expand = if s.len() == 3 {
        s.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        s.to_string()
    };
    if expand.len() != 6 || !expand.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("not a 3- or 6-digit hex color: {}", raw));
    }
    let r = u8::from_str_radix(&expand[0..2], 16).map_err(|e| e.to_string())?;
    let g = u8::from_str_radix(&expand[2..4], 16).map_err(|e| e.to_string())?;
    let b = u8::from_str_radix(&expand[4..6], 16).map_err(|e| e.to_string())?;
    Ok((r, g, b))
}

fn parse_rgb(raw: &str) -> Result<(u8, u8, u8), String> {
    let s = raw.trim();
    let inner = s
        .strip_prefix("rgb(")
        .and_then(|x| x.strip_suffix(')'))
        .ok_or_else(|| format!("expected rgb(R, G, B): {}", raw))?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        return Err("rgb() needs 3 components".into());
    }
    let v: Vec<u16> = parts
        .iter()
        .map(|p| p.parse::<u16>().map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    if v.iter().any(|x| *x > 255) {
        return Err("rgb component out of 0..=255".into());
    }
    Ok((v[0] as u8, v[1] as u8, v[2] as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn six_digit_hex_to_rgb() {
        let tool = ColorHexRgb;
        let r = tool
            .call(&json!({"op": "to_rgb", "hex": "#FF4000"}))
            .await
            .unwrap();
        assert_eq!(r["rgb"], "rgb(255, 64, 0)");
        assert_eq!(r["r"], 255);
        assert_eq!(r["g"], 64);
        assert_eq!(r["b"], 0);
    }

    #[tokio::test]
    async fn three_digit_hex_expands() {
        let tool = ColorHexRgb;
        let r = tool
            .call(&json!({"op": "to_rgb", "hex": "f80"}))
            .await
            .unwrap();
        // #f80 → #ff8800
        assert_eq!(r["r"], 0xff);
        assert_eq!(r["g"], 0x88);
        assert_eq!(r["b"], 0x00);
    }

    #[tokio::test]
    async fn rgb_to_hex_round_trip() {
        let tool = ColorHexRgb;
        let r = tool
            .call(&json!({"op": "to_hex", "rgb": "rgb(255, 64, 0)"}))
            .await
            .unwrap();
        assert_eq!(r["hex"], "#FF4000");
    }
}
