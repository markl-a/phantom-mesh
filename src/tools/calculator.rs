use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use super::{Tool, ToolResult};

pub struct CalculatorTool;
impl CalculatorTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl Tool for CalculatorTool {
    fn name(&self) -> &str { "calculator" }
    fn description(&self) -> &str { "Evaluate mathematical expressions and perform unit conversions" }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {"type": "string", "enum": ["evaluate", "convert"]},
                "expression": {"type": "string"},
                "to_unit": {"type": "string"}
            },
            "required": ["operation", "expression"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let operation = args["operation"].as_str().unwrap_or("evaluate");
        let expression = args["expression"].as_str().unwrap_or("");
        match operation {
            "evaluate" => evaluate_expression(expression),
            "convert" => convert_unit(expression, args["to_unit"].as_str().unwrap_or("")),
            _ => Ok(ToolResult { output: format!("Unknown operation: {}", operation), success: false }),
        }
    }
}

fn evaluate_expression(expr: &str) -> Result<ToolResult> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Ok(ToolResult { output: "Empty expression".to_string(), success: false });
    }
    match parse_and_eval(expr) {
        Ok(result) => Ok(ToolResult {
            output: json!({"expression": expr, "result": result, "formatted": format_number(result)}).to_string(),
            success: true,
        }),
        Err(e) => Ok(ToolResult {
            output: format!("Failed to evaluate: {}", e),
            success: false,
        }),
    }
}

fn parse_and_eval(expr: &str) -> Result<f64> {
    let expr = expr.replace(' ', "");
    let tokens = tokenize(&expr)?;
    let mut pos = 0;
    let result = parse_addition(&tokens, &mut pos)?;
    if pos < tokens.len() { anyhow::bail!("Unexpected token"); }
    Ok(result)
}

#[derive(Debug, Clone)]
enum Token {
    Number(f64), Plus, Minus, Multiply, Divide, Power, LParen, RParen, Function(String),
}

fn tokenize(expr: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '+' => { tokens.push(Token::Plus); i += 1; }
            '-' => {
                if tokens.is_empty() || matches!(tokens.last(),
                    Some(Token::LParen) | Some(Token::Plus) | Some(Token::Minus) |
                    Some(Token::Multiply) | Some(Token::Divide) | Some(Token::Power))
                {
                    let mut num_str = String::from("-");
                    i += 1;
                    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                        num_str.push(chars[i]); i += 1;
                    }
                    if num_str.len() > 1 {
                        tokens.push(Token::Number(num_str.parse()?));
                    } else {
                        anyhow::bail!("Invalid negative number");
                    }
                } else {
                    tokens.push(Token::Minus); i += 1;
                }
            }
            '*' => { tokens.push(Token::Multiply); i += 1; }
            '/' => { tokens.push(Token::Divide); i += 1; }
            '^' => { tokens.push(Token::Power); i += 1; }
            '(' => { tokens.push(Token::LParen); i += 1; }
            ')' => { tokens.push(Token::RParen); i += 1; }
            c if c.is_ascii_digit() || c == '.' => {
                let mut s = String::new();
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    s.push(chars[i]); i += 1;
                }
                tokens.push(Token::Number(s.parse()?));
            }
            c if c.is_ascii_alphabetic() => {
                let mut f = String::new();
                while i < chars.len() && chars[i].is_ascii_alphabetic() {
                    f.push(chars[i]); i += 1;
                }
                match f.as_str() {
                    "pi" => tokens.push(Token::Number(std::f64::consts::PI)),
                    "e" => tokens.push(Token::Number(std::f64::consts::E)),
                    _ => tokens.push(Token::Function(f)),
                }
            }
            _ => anyhow::bail!("Unexpected character: {}", chars[i]),
        }
    }
    Ok(tokens)
}

fn parse_addition(tokens: &[Token], pos: &mut usize) -> Result<f64> {
    let mut result = parse_multiplication(tokens, pos)?;
    while *pos < tokens.len() {
        match &tokens[*pos] {
            Token::Plus => { *pos += 1; result += parse_multiplication(tokens, pos)?; }
            Token::Minus => { *pos += 1; result -= parse_multiplication(tokens, pos)?; }
            _ => break,
        }
    }
    Ok(result)
}

fn parse_multiplication(tokens: &[Token], pos: &mut usize) -> Result<f64> {
    let mut result = parse_power(tokens, pos)?;
    while *pos < tokens.len() {
        match &tokens[*pos] {
            Token::Multiply => { *pos += 1; result *= parse_power(tokens, pos)?; }
            Token::Divide => {
                *pos += 1;
                let d = parse_power(tokens, pos)?;
                if d == 0.0 { anyhow::bail!("Division by zero"); }
                result /= d;
            }
            _ => break,
        }
    }
    Ok(result)
}

fn parse_power(tokens: &[Token], pos: &mut usize) -> Result<f64> {
    let base = parse_unary(tokens, pos)?;
    if *pos < tokens.len() {
        if let Token::Power = &tokens[*pos] {
            *pos += 1;
            let exp = parse_power(tokens, pos)?;
            return Ok(base.powf(exp));
        }
    }
    Ok(base)
}

fn parse_unary(tokens: &[Token], pos: &mut usize) -> Result<f64> {
    if *pos >= tokens.len() { anyhow::bail!("Unexpected end of expression"); }
    match &tokens[*pos] {
        Token::Function(name) => {
            let func_name = name.clone();
            *pos += 1;
            if *pos >= tokens.len() || !matches!(&tokens[*pos], Token::LParen) {
                anyhow::bail!("Expected '(' after function {}", func_name);
            }
            *pos += 1;
            let arg = parse_addition(tokens, pos)?;
            if *pos >= tokens.len() || !matches!(&tokens[*pos], Token::RParen) {
                anyhow::bail!("Expected ')' after function argument");
            }
            *pos += 1;
            match func_name.as_str() {
                "sqrt" => Ok(arg.sqrt()),
                "abs" => Ok(arg.abs()),
                "sin" => Ok(arg.sin()),
                "cos" => Ok(arg.cos()),
                "tan" => Ok(arg.tan()),
                "ln" => Ok(arg.ln()),
                "log" => Ok(arg.log10()),
                "ceil" => Ok(arg.ceil()),
                "floor" => Ok(arg.floor()),
                "round" => Ok(arg.round()),
                _ => anyhow::bail!("Unknown function: {}", func_name),
            }
        }
        Token::LParen => {
            *pos += 1;
            let r = parse_addition(tokens, pos)?;
            if *pos >= tokens.len() || !matches!(&tokens[*pos], Token::RParen) {
                anyhow::bail!("Mismatched parentheses");
            }
            *pos += 1;
            Ok(r)
        }
        Token::Number(n) => { let v = *n; *pos += 1; Ok(v) }
        _ => anyhow::bail!("Unexpected token at position {}", pos),
    }
}

fn format_number(n: f64) -> String {
    if n == n.floor() && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{:.6}", n).trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn convert_unit(expression: &str, to_unit: &str) -> Result<ToolResult> {
    let parts: Vec<&str> = expression.trim().splitn(2, char::is_whitespace).collect();
    if parts.len() != 2 {
        return Ok(ToolResult { output: "Expected format: '<number> <unit>'".to_string(), success: false });
    }
    let value: f64 = parts[0].parse().map_err(|_| anyhow::anyhow!("Invalid number: {}", parts[0]))?;
    let from_unit = parts[1].trim().to_lowercase();
    let to = to_unit.trim().to_lowercase();
    let result = do_conversion(value, &from_unit, &to)?;
    Ok(ToolResult {
        output: json!({"from": format!("{} {}", value, from_unit), "to": format!("{} {}", format_number(result), to), "result": result}).to_string(),
        success: true,
    })
}

fn do_conversion(value: f64, from: &str, to: &str) -> Result<f64> {
    let length: &[(&str, f64)] = &[
        ("m", 1.0), ("km", 1000.0), ("cm", 0.01), ("mm", 0.001),
        ("mi", 1609.344), ("yd", 0.9144), ("ft", 0.3048), ("in", 0.0254),
    ];
    let weight: &[(&str, f64)] = &[
        ("g", 1.0), ("kg", 1000.0), ("mg", 0.001),
        ("lb", 453.592), ("oz", 28.3495), ("ton", 907185.0), ("tonne", 1_000_000.0),
    ];
    // Temperature
    if (from == "c" || from == "celsius") && (to == "f" || to == "fahrenheit") { return Ok(value * 9.0 / 5.0 + 32.0); }
    if (from == "f" || from == "fahrenheit") && (to == "c" || to == "celsius") { return Ok((value - 32.0) * 5.0 / 9.0); }
    if (from == "c" || from == "celsius") && (to == "k" || to == "kelvin") { return Ok(value + 273.15); }
    if (from == "k" || from == "kelvin") && (to == "c" || to == "celsius") { return Ok(value - 273.15); }
    // Length
    if let (Some(ff), Some(tf)) = (
        length.iter().find(|(u, _)| *u == from).map(|(_, f)| f),
        length.iter().find(|(u, _)| *u == to).map(|(_, f)| f),
    ) { return Ok(value * ff / tf); }
    // Weight
    if let (Some(ff), Some(tf)) = (
        weight.iter().find(|(u, _)| *u == from).map(|(_, f)| f),
        weight.iter().find(|(u, _)| *u == to).map(|(_, f)| f),
    ) { return Ok(value * ff / tf); }
    anyhow::bail!("Cannot convert from '{}' to '{}'", from, to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_arithmetic() {
        let t = CalculatorTool::new();
        let r = t.execute(json!({"operation": "evaluate", "expression": "2+3*4"})).await.unwrap();
        assert!(r.success);
        let p: Value = serde_json::from_str(&r.output).unwrap();
        assert_eq!(p["result"].as_f64().unwrap(), 14.0);
    }

    #[tokio::test]
    async fn test_parentheses() {
        let t = CalculatorTool::new();
        let r = t.execute(json!({"operation": "evaluate", "expression": "(2+3)*4"})).await.unwrap();
        let p: Value = serde_json::from_str(&r.output).unwrap();
        assert_eq!(p["result"].as_f64().unwrap(), 20.0);
    }

    #[tokio::test]
    async fn test_power() {
        let t = CalculatorTool::new();
        let r = t.execute(json!({"operation": "evaluate", "expression": "2^10"})).await.unwrap();
        let p: Value = serde_json::from_str(&r.output).unwrap();
        assert_eq!(p["result"].as_f64().unwrap(), 1024.0);
    }

    #[tokio::test]
    async fn test_sqrt() {
        let t = CalculatorTool::new();
        let r = t.execute(json!({"operation": "evaluate", "expression": "sqrt(144)"})).await.unwrap();
        let p: Value = serde_json::from_str(&r.output).unwrap();
        assert_eq!(p["result"].as_f64().unwrap(), 12.0);
    }

    #[tokio::test]
    async fn test_division_by_zero() {
        let t = CalculatorTool::new();
        let r = t.execute(json!({"operation": "evaluate", "expression": "1/0"})).await.unwrap();
        assert!(!r.success);
    }

    #[tokio::test]
    async fn test_negative() {
        let t = CalculatorTool::new();
        let r = t.execute(json!({"operation": "evaluate", "expression": "-5+3"})).await.unwrap();
        let p: Value = serde_json::from_str(&r.output).unwrap();
        assert_eq!(p["result"].as_f64().unwrap(), -2.0);
    }

    #[tokio::test]
    async fn test_pi() {
        let t = CalculatorTool::new();
        let r = t.execute(json!({"operation": "evaluate", "expression": "pi*2"})).await.unwrap();
        let p: Value = serde_json::from_str(&r.output).unwrap();
        assert!((p["result"].as_f64().unwrap() - std::f64::consts::PI * 2.0).abs() < 1e-10);
    }

    #[tokio::test]
    async fn test_km_to_mi() {
        let t = CalculatorTool::new();
        let r = t.execute(json!({"operation": "convert", "expression": "100 km", "to_unit": "mi"})).await.unwrap();
        let p: Value = serde_json::from_str(&r.output).unwrap();
        assert!((p["result"].as_f64().unwrap() - 62.1371).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_c_to_f() {
        let t = CalculatorTool::new();
        let r = t.execute(json!({"operation": "convert", "expression": "100 c", "to_unit": "f"})).await.unwrap();
        let p: Value = serde_json::from_str(&r.output).unwrap();
        assert_eq!(p["result"].as_f64().unwrap(), 212.0);
    }

    #[tokio::test]
    async fn test_kg_to_lb() {
        let t = CalculatorTool::new();
        let r = t.execute(json!({"operation": "convert", "expression": "1 kg", "to_unit": "lb"})).await.unwrap();
        let p: Value = serde_json::from_str(&r.output).unwrap();
        assert!((p["result"].as_f64().unwrap() - 2.20462).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_empty_expression() {
        let t = CalculatorTool::new();
        let r = t.execute(json!({"operation": "evaluate", "expression": ""})).await.unwrap();
        assert!(!r.success);
    }

    #[tokio::test]
    async fn test_invalid_convert_format() {
        let t = CalculatorTool::new();
        let r = t.execute(json!({"operation": "convert", "expression": "100", "to_unit": "km"})).await.unwrap();
        assert!(!r.success);
    }

    #[test]
    fn test_format_number_integer() {
        assert_eq!(format_number(42.0), "42");
        assert_eq!(format_number(-7.0), "-7");
    }

    #[test]
    fn test_format_number_decimal() {
        assert_eq!(format_number(3.14159), "3.14159");
    }

    #[test]
    fn test_name_and_description() {
        let t = CalculatorTool::new();
        assert_eq!(t.name(), "calculator");
        assert!(!t.description().is_empty());
        let schema = t.parameters_schema();
        assert!(schema["properties"]["operation"].is_object());
    }
}
