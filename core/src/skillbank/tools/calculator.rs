//! `skill_calculator` — safe arithmetic expression evaluator.
//!
//! Implementation is a fresh shunting-yard pass with no external CAS
//! and no shell-out — safe for untrusted input.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{SkillTool, ToolError, ToolResult};

pub struct Calculator;

#[async_trait]
impl SkillTool for Calculator {
    fn name(&self) -> &'static str {
        "skill_calculator"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "skill_calculator",
                "description": "Evaluate a numeric expression with + - * / % and parentheses. \
                    No variables, no functions, no shell-out — pure parser, safe for untrusted input.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "expression": {
                            "type": "string",
                            "description": "Expression to evaluate, e.g. '2 + 3 * (4 - 1)'."
                        }
                    },
                    "required": ["expression"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let expr = args
            .get("expression")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("expression required".into()))?;
        let result = evaluate(expr).map_err(ToolError::Invalid)?;
        Ok(json!({ "result": result }))
    }
}

/// Shunting-yard evaluator. Returns the numeric result or an error
/// string suitable for `ToolError::Invalid`.
pub(crate) fn evaluate(expr: &str) -> Result<f64, String> {
    // Tokenise.
    let mut tokens: Vec<Token> = Vec::new();
    let mut chars = expr.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        if c.is_ascii_digit() || c == '.' {
            let mut num = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() || d == '.' {
                    num.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            let n: f64 = num.parse().map_err(|_| format!("not a number: {}", num))?;
            tokens.push(Token::Num(n));
        } else if matches!(c, '+' | '-' | '*' | '/' | '%' | '(' | ')') {
            // Unary minus: `-x` or after operator/`(`. Convert to `0 - x`.
            if c == '-'
                && (tokens.is_empty()
                    || matches!(tokens.last(), Some(Token::Op(_)) | Some(Token::LParen)))
            {
                tokens.push(Token::Num(0.0));
            }
            tokens.push(match c {
                '(' => Token::LParen,
                ')' => Token::RParen,
                op => Token::Op(op),
            });
            chars.next();
        } else {
            return Err(format!("unexpected char: {}", c));
        }
    }
    // Shunting-yard → RPN.
    let mut output: Vec<Token> = Vec::new();
    let mut ops: Vec<Token> = Vec::new();
    for t in tokens {
        match t {
            Token::Num(_) => output.push(t),
            Token::Op(op) => {
                while let Some(Token::Op(top)) = ops.last() {
                    if precedence(*top) >= precedence(op) {
                        output.push(ops.pop().unwrap());
                    } else {
                        break;
                    }
                }
                ops.push(Token::Op(op));
            }
            Token::LParen => ops.push(Token::LParen),
            Token::RParen => {
                while let Some(t) = ops.pop() {
                    if matches!(t, Token::LParen) {
                        break;
                    }
                    output.push(t);
                }
            }
        }
    }
    while let Some(t) = ops.pop() {
        output.push(t);
    }
    // Eval RPN.
    let mut stack: Vec<f64> = Vec::new();
    for t in output {
        match t {
            Token::Num(n) => stack.push(n),
            Token::Op(op) => {
                let b = stack.pop().ok_or_else(|| "stack underflow".to_string())?;
                let a = stack.pop().ok_or_else(|| "stack underflow".to_string())?;
                let r = match op {
                    '+' => a + b,
                    '-' => a - b,
                    '*' => a * b,
                    '/' => {
                        if b == 0.0 {
                            return Err("division by zero".into());
                        } else {
                            a / b
                        }
                    }
                    '%' => {
                        if b == 0.0 {
                            return Err("modulo by zero".into());
                        } else {
                            a % b
                        }
                    }
                    _ => return Err(format!("bad op: {}", op)),
                };
                stack.push(r);
            }
            _ => return Err("malformed expression".into()),
        }
    }
    stack.pop().ok_or_else(|| "empty expression".to_string())
}

#[derive(Clone, Copy)]
enum Token {
    Num(f64),
    Op(char),
    LParen,
    RParen,
}

fn precedence(op: char) -> u8 {
    match op {
        '+' | '-' => 1,
        '*' | '/' | '%' => 2,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn evaluates_basic_arithmetic() {
        let tool = Calculator;
        let r = tool
            .call(&json!({"expression": "2 + 3 * 4"}))
            .await
            .unwrap();
        assert_eq!(r["result"], 14.0);
    }

    #[tokio::test]
    async fn respects_parentheses_and_unary_minus() {
        let tool = Calculator;
        let r = tool
            .call(&json!({"expression": "-(2 + 3) * 4"}))
            .await
            .unwrap();
        assert_eq!(r["result"], -20.0);
    }

    #[tokio::test]
    async fn division_by_zero_is_invalid_not_panic() {
        let tool = Calculator;
        let err = tool
            .call(&json!({"expression": "1 / 0"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Invalid(_)));
    }

    #[tokio::test]
    async fn missing_expression_is_bad_args() {
        let tool = Calculator;
        let err = tool.call(&json!({})).await.unwrap_err();
        assert!(matches!(err, ToolError::BadArgs(_)));
    }
}
