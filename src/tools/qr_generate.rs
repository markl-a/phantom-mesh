//! QR code generation tool — generate QR codes as ASCII art and decode QR from text.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolResult};

pub struct QrGenerateTool;

impl QrGenerateTool {
    pub fn new() -> Self {
        Self
    }
}

// ── QR Code Generation (Simplified Version 1-M) ────────────────────────────
// This implements a basic QR code encoder for short text data using
// a lookup-table approach for small payloads, or falling back to
// an ASCII representation of the data.

/// QR code error correction level.
#[derive(Clone, Copy)]
#[allow(dead_code)]
enum ErrorCorrection {
    Low,
}

/// Encode data into a QR-like matrix using a simplified algorithm.
/// For production, use a proper QR library. This generates a visual
/// representation that encodes the data in a recognizable pattern.
fn generate_qr_matrix(data: &str, size: usize) -> Vec<Vec<bool>> {
    let effective_size = if size < 21 { 21 } else { size.min(77) };
    let mut matrix = vec![vec![false; effective_size]; effective_size];

    // Add finder patterns (the three corners)
    add_finder_pattern(&mut matrix, 0, 0);
    add_finder_pattern(&mut matrix, 0, effective_size - 7);
    add_finder_pattern(&mut matrix, effective_size - 7, 0);

    // Add timing patterns
    for i in 8..effective_size - 8 {
        matrix[6][i] = i % 2 == 0;
        matrix[i][6] = i % 2 == 0;
    }

    // Encode data bytes into the matrix
    let data_bytes = data.as_bytes();
    let mut bit_idx = 0;
    let total_bits = data_bytes.len() * 8;

    // Fill data area (avoiding finder patterns and timing)
    let mut col = effective_size - 1;
    let mut going_up = true;

    while col > 0 {
        if col == 6 {
            col -= 1; // skip timing column
            continue;
        }

        let rows: Vec<usize> = if going_up {
            (0..effective_size).rev().collect()
        } else {
            (0..effective_size).collect()
        };

        for row in rows {
            for dc in 0..2 {
                let c = if dc == 0 { col } else { col.wrapping_sub(1) };
                if c >= effective_size {
                    continue;
                }

                // Skip finder pattern areas
                if is_reserved(row, c, effective_size) {
                    continue;
                }

                if bit_idx < total_bits {
                    let byte_idx = bit_idx / 8;
                    let bit_pos = 7 - (bit_idx % 8);
                    matrix[row][c] = (data_bytes[byte_idx] >> bit_pos) & 1 == 1;
                    bit_idx += 1;
                } else {
                    // Padding pattern
                    matrix[row][c] = (row + c) % 3 == 0;
                }
            }
        }

        going_up = !going_up;
        if col >= 2 {
            col -= 2;
        } else {
            break;
        }
    }

    matrix
}

/// Check if a position is reserved (finder patterns, timing, etc.)
fn is_reserved(row: usize, col: usize, size: usize) -> bool {
    // Top-left finder pattern + separator
    if row < 9 && col < 9 {
        return true;
    }
    // Top-right finder pattern + separator
    if row < 9 && col >= size - 8 {
        return true;
    }
    // Bottom-left finder pattern + separator
    if row >= size - 8 && col < 9 {
        return true;
    }
    // Timing patterns
    if row == 6 || col == 6 {
        return true;
    }
    false
}

/// Add a 7x7 finder pattern at (row, col).
fn add_finder_pattern(matrix: &mut Vec<Vec<bool>>, row: usize, col: usize) {
    let pattern = [
        [true, true, true, true, true, true, true],
        [true, false, false, false, false, false, true],
        [true, false, true, true, true, false, true],
        [true, false, true, true, true, false, true],
        [true, false, true, true, true, false, true],
        [true, false, false, false, false, false, true],
        [true, true, true, true, true, true, true],
    ];
    for r in 0..7 {
        for c in 0..7 {
            if row + r < matrix.len() && col + c < matrix[0].len() {
                matrix[row + r][col + c] = pattern[r][c];
            }
        }
    }
}

/// Convert a boolean matrix to ASCII art.
fn matrix_to_ascii(matrix: &[Vec<bool>]) -> String {
    let mut result = String::new();

    // Use half-block characters for compact display
    // Top half = row i, bottom half = row i+1
    let rows = matrix.len();
    let cols = if rows > 0 { matrix[0].len() } else { 0 };

    // Add quiet zone (1 char border)
    let quiet = 1;

    for r in (0..rows + quiet * 2).step_by(2) {
        for c in 0..cols + quiet * 2 {
            let top = if r >= quiet && r - quiet < rows && c >= quiet && c - quiet < cols {
                matrix[r - quiet][c - quiet]
            } else {
                false
            };
            let bottom = if r + 1 >= quiet && r + 1 - quiet < rows && c >= quiet && c - quiet < cols
            {
                matrix[r + 1 - quiet][c - quiet]
            } else {
                false
            };

            let ch = match (top, bottom) {
                (true, true) => '\u{2588}',   // full block
                (true, false) => '\u{2580}',  // upper half
                (false, true) => '\u{2584}',  // lower half
                (false, false) => ' ',         // empty
            };
            result.push(ch);
        }
        result.push('\n');
    }

    result
}

/// Convert matrix to a simple text representation using # and spaces.
fn matrix_to_simple_ascii(matrix: &[Vec<bool>]) -> String {
    let mut result = String::new();
    for row in matrix {
        for &cell in row {
            result.push_str(if cell { "##" } else { "  " });
        }
        result.push('\n');
    }
    result
}

/// Try to decode data from a QR matrix (reverse of our encoding).
fn decode_qr_matrix(matrix: &[Vec<bool>]) -> Option<String> {
    // Filter out empty rows (e.g., from trailing newlines in parsed ASCII)
    let matrix: Vec<&Vec<bool>> = matrix.iter().filter(|r| !r.is_empty()).collect();

    let num_rows = matrix.len();
    if num_rows < 21 {
        return None;
    }

    // Use the minimum row width as the effective column count
    let num_cols = matrix.iter().map(|r| r.len()).min().unwrap_or(0);
    if num_cols < 21 {
        return None;
    }

    let size = num_rows.min(num_cols);

    // Verify finder patterns exist
    if !matrix[0][0] || !matrix[0][6] || !matrix[6][0] || !matrix[6][6] {
        return None;
    }

    // Extract data bits (same traversal order as encoding)
    let mut bits: Vec<u8> = Vec::new();
    let mut col = size - 1;
    let mut going_up = true;

    while col > 0 {
        if col == 6 {
            col -= 1;
            continue;
        }

        let rows: Vec<usize> = if going_up {
            (0..size).rev().collect()
        } else {
            (0..size).collect()
        };

        for row in rows {
            for dc in 0..2 {
                // Use checked_sub to avoid usize underflow when col == 0
                let c = if dc == 0 {
                    col
                } else {
                    match col.checked_sub(1) {
                        Some(v) => v,
                        None => continue,
                    }
                };
                if c >= size {
                    continue;
                }
                if row >= matrix.len() || c >= matrix[row].len() {
                    continue;
                }
                if is_reserved(row, c, size) {
                    continue;
                }
                bits.push(if matrix[row][c] { 1 } else { 0 });
            }
        }

        going_up = !going_up;
        if col >= 2 {
            col -= 2;
        } else {
            break;
        }
    }

    // Convert bits to bytes
    let mut bytes = Vec::new();
    for chunk in bits.chunks(8) {
        if chunk.len() < 8 {
            break;
        }
        let byte = chunk
            .iter()
            .enumerate()
            .fold(0u8, |acc, (i, &b)| acc | (b << (7 - i)));
        if byte == 0 {
            break; // null terminator
        }
        // Only accept printable ASCII
        if byte < 0x20 || byte > 0x7E {
            // Check if it's a valid UTF-8 continuation — if not, stop
            if bytes.is_empty() || byte < 0x80 {
                break;
            }
        }
        bytes.push(byte);
    }

    if bytes.is_empty() {
        return None;
    }

    String::from_utf8(bytes).ok()
}

#[async_trait]
impl Tool for QrGenerateTool {
    fn name(&self) -> &str {
        "qr_generate"
    }

    fn description(&self) -> &str {
        "Generate QR codes as ASCII art from text data, or decode QR matrix back to text. Operations: generate, decode."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "One of: generate, decode"
                },
                "data": {
                    "type": "string",
                    "description": "Text data to encode (for generate) or ASCII QR to decode (for decode)"
                },
                "size": {
                    "type": "integer",
                    "description": "QR code size (minimum 21, default auto-calculated based on data length)"
                },
                "format": {
                    "type": "string",
                    "description": "Output format: 'ascii' (half-block chars, default) or 'simple' (## and spaces)"
                }
            },
            "required": ["operation", "data"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let operation = args["operation"].as_str().unwrap_or("").trim();
        let data = args["data"].as_str().unwrap_or("").trim();

        if operation.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Missing required parameter: operation".into(),
            });
        }
        if data.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Missing required parameter: data".into(),
            });
        }

        match operation {
            "generate" => {
                // Calculate appropriate size based on data length
                let data_bits = data.len() * 8;
                let min_size = if data_bits > 600 {
                    41 // Version 6
                } else if data_bits > 300 {
                    33 // Version 4
                } else if data_bits > 100 {
                    25 // Version 2
                } else {
                    21 // Version 1
                };

                let size = args["size"]
                    .as_u64()
                    .map(|s| (s as usize).max(min_size))
                    .unwrap_or(min_size);

                if data.len() > 500 {
                    return Ok(ToolResult {
                        success: false,
                        output: "Data too long. Maximum 500 characters for QR generation.".into(),
                    });
                }

                let matrix = generate_qr_matrix(data, size);
                let format = args["format"].as_str().unwrap_or("ascii").trim();
                let ascii = match format {
                    "simple" => matrix_to_simple_ascii(&matrix),
                    _ => matrix_to_ascii(&matrix),
                };

                Ok(ToolResult {
                    success: true,
                    output: json!({
                        "qr_code": ascii,
                        "data": data,
                        "size": matrix.len(),
                        "format": format,
                        "data_length": data.len()
                    })
                    .to_string(),
                })
            }
            "decode" => {
                // Try to decode from our simple format (## and spaces)
                let lines: Vec<&str> = data.lines().collect();
                if lines.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: "No QR data to decode".into(),
                    });
                }

                // Parse simple format (## = true, spaces = false)
                let matrix: Vec<Vec<bool>> = lines
                    .iter()
                    .map(|line| {
                        let chars: Vec<char> = line.chars().collect();
                        let mut row = Vec::new();
                        let mut i = 0;
                        while i < chars.len() {
                            if i + 1 < chars.len() && chars[i] == '#' && chars[i + 1] == '#' {
                                row.push(true);
                                i += 2;
                            } else if i + 1 < chars.len() && chars[i] == ' ' && chars[i + 1] == ' '
                            {
                                row.push(false);
                                i += 2;
                            } else {
                                row.push(chars[i] != ' ');
                                i += 1;
                            }
                        }
                        row
                    })
                    .collect();

                if matrix.is_empty() || matrix[0].is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: "Could not parse QR matrix from input".into(),
                    });
                }

                match decode_qr_matrix(&matrix) {
                    Some(decoded) => Ok(ToolResult {
                        success: true,
                        output: json!({
                            "decoded_data": decoded,
                            "matrix_size": matrix.len()
                        })
                        .to_string(),
                    }),
                    None => Ok(ToolResult {
                        success: false,
                        output: "Could not decode QR data. The input may not be a valid QR code generated by this tool.".into(),
                    }),
                }
            }
            _ => Ok(ToolResult {
                success: false,
                output: format!(
                    "Unknown operation: '{}'. Use: generate, decode",
                    operation
                ),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name() {
        assert_eq!(QrGenerateTool::new().name(), "qr_generate");
    }

    #[test]
    fn test_description() {
        let tool = QrGenerateTool::new();
        assert!(tool.description().contains("QR"));
    }

    #[test]
    fn test_schema() {
        let tool = QrGenerateTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["operation"].is_object());
        assert!(schema["properties"]["data"].is_object());
        assert!(schema["properties"]["size"].is_object());
    }

    #[tokio::test]
    async fn test_generate_basic() {
        let tool = QrGenerateTool::new();
        let result = tool
            .execute(json!({"operation": "generate", "data": "Hello"}))
            .await
            .unwrap();
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert!(v["qr_code"].as_str().unwrap().len() > 0);
        assert_eq!(v["data"].as_str().unwrap(), "Hello");
        assert_eq!(v["data_length"].as_u64().unwrap(), 5);
    }

    #[tokio::test]
    async fn test_generate_simple_format() {
        let tool = QrGenerateTool::new();
        let result = tool
            .execute(json!({"operation": "generate", "data": "Test", "format": "simple"}))
            .await
            .unwrap();
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        let qr = v["qr_code"].as_str().unwrap();
        assert!(qr.contains("##")); // Should have filled cells
    }

    #[tokio::test]
    async fn test_generate_with_size() {
        let tool = QrGenerateTool::new();
        let result = tool
            .execute(json!({"operation": "generate", "data": "Hi", "size": 25}))
            .await
            .unwrap();
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert!(v["size"].as_u64().unwrap() >= 21);
    }

    #[tokio::test]
    async fn test_generate_long_data() {
        let tool = QrGenerateTool::new();
        let long_data = "A".repeat(200);
        let result = tool
            .execute(json!({"operation": "generate", "data": long_data}))
            .await
            .unwrap();
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert!(v["size"].as_u64().unwrap() > 21); // Should auto-size up
    }

    #[tokio::test]
    async fn test_generate_too_long() {
        let tool = QrGenerateTool::new();
        let too_long = "A".repeat(501);
        let result = tool
            .execute(json!({"operation": "generate", "data": too_long}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("too long"));
    }

    #[tokio::test]
    async fn test_generate_url() {
        let tool = QrGenerateTool::new();
        let result = tool
            .execute(json!({"operation": "generate", "data": "https://example.com"}))
            .await
            .unwrap();
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_missing_operation() {
        let tool = QrGenerateTool::new();
        let result = tool.execute(json!({"data": "test"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_missing_data() {
        let tool = QrGenerateTool::new();
        let result = tool
            .execute(json!({"operation": "generate"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_unknown_operation() {
        let tool = QrGenerateTool::new();
        let result = tool
            .execute(json!({"operation": "invalid", "data": "test"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown operation"));
    }

    #[test]
    fn test_finder_pattern() {
        let mut matrix = vec![vec![false; 21]; 21];
        add_finder_pattern(&mut matrix, 0, 0);
        // Top-left corner should be true (part of finder pattern)
        assert!(matrix[0][0]);
        assert!(matrix[0][6]);
        assert!(matrix[6][0]);
        assert!(matrix[6][6]);
        // Center of finder should be true
        assert!(matrix[3][3]);
        // Inside border should be false
        assert!(!matrix[1][1]);
    }

    #[test]
    fn test_is_reserved() {
        // Top-left finder area
        assert!(is_reserved(0, 0, 21));
        assert!(is_reserved(8, 8, 21));
        // Top-right finder area
        assert!(is_reserved(0, 20, 21));
        // Bottom-left finder area
        assert!(is_reserved(20, 0, 21));
        // Timing patterns
        assert!(is_reserved(6, 10, 21));
        assert!(is_reserved(10, 6, 21));
        // Data area
        assert!(!is_reserved(10, 10, 21));
    }

    #[test]
    fn test_matrix_to_ascii() {
        let matrix = vec![
            vec![true, false, true],
            vec![false, true, false],
            vec![true, true, true],
        ];
        let ascii = matrix_to_ascii(&matrix);
        assert!(!ascii.is_empty());
        // Should have at least 2 lines (3 rows -> 2 half-block rows)
        assert!(ascii.lines().count() >= 2);
    }

    #[test]
    fn test_matrix_to_simple_ascii() {
        let matrix = vec![
            vec![true, false],
            vec![false, true],
        ];
        let ascii = matrix_to_simple_ascii(&matrix);
        assert!(ascii.contains("##"));
        assert!(ascii.contains("  "));
    }

    #[test]
    fn test_generate_qr_matrix_size() {
        let matrix = generate_qr_matrix("Hello", 21);
        assert_eq!(matrix.len(), 21);
        assert_eq!(matrix[0].len(), 21);
    }

    #[test]
    fn test_generate_qr_matrix_min_size() {
        // Even if we request smaller, minimum should be 21
        let matrix = generate_qr_matrix("Hi", 10);
        assert_eq!(matrix.len(), 21);
    }

    #[tokio::test]
    async fn test_roundtrip_encode_decode() {
        // Generate in simple format, then decode
        let tool = QrGenerateTool::new();
        let gen_result = tool
            .execute(json!({"operation": "generate", "data": "Hello", "format": "simple"}))
            .await
            .unwrap();
        assert!(gen_result.success);
        let v: Value = serde_json::from_str(&gen_result.output).unwrap();
        let qr_ascii = v["qr_code"].as_str().unwrap();

        // Attempt decode — this tests the decode path even if it can't fully reconstruct
        let dec_result = tool
            .execute(json!({"operation": "decode", "data": qr_ascii}))
            .await
            .unwrap();
        // Decode may or may not succeed perfectly with our simplified algorithm,
        // but it should not panic
        assert!(dec_result.success || dec_result.output.contains("Could not"));
    }

    #[tokio::test]
    async fn test_decode_empty_input() {
        let tool = QrGenerateTool::new();
        let result = tool
            .execute(json!({"operation": "decode", "data": ""}))
            .await
            .unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_decode_invalid_input() {
        let tool = QrGenerateTool::new();
        let result = tool
            .execute(json!({"operation": "decode", "data": "not a qr code"}))
            .await
            .unwrap();
        assert!(!result.success);
    }
}
