//! AWS Bedrock provider — behind `bedrock` feature flag.
//! Uses the Converse API, not OpenAI-compat.

use std::collections::HashMap;

/// Known Bedrock model ID mappings for short names
fn bedrock_model_map() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert(
        "claude-3-sonnet",
        "anthropic.claude-3-sonnet-20240229-v1:0",
    );
    m.insert(
        "claude-3-haiku",
        "anthropic.claude-3-haiku-20240307-v1:0",
    );
    m.insert(
        "claude-3-opus",
        "anthropic.claude-3-opus-20240229-v1:0",
    );
    m.insert(
        "claude-3.5-sonnet",
        "anthropic.claude-3-5-sonnet-20241022-v2:0",
    );
    m
}

pub fn normalize_bedrock_model(model: &str) -> String {
    if model.contains('.') {
        // Already qualified (e.g., "anthropic.claude-3-sonnet-...")
        return model.to_string();
    }
    bedrock_model_map()
        .get(model)
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("anthropic.{}", model))
}

pub struct BedrockProvider {
    pub region: String,
}

impl BedrockProvider {
    pub fn new(region: String) -> Self {
        Self { region }
    }

    pub fn provider_name() -> &'static str {
        "bedrock"
    }
}

// Full AWS SDK integration requires the `bedrock` feature flag.
// The Converse API implementation is added when building with:
//   cargo build --features bedrock
#[cfg(feature = "bedrock")]
mod sdk_impl {
    use super::*;
    use anyhow::Result;

    impl BedrockProvider {
        pub async fn init_client(&self) -> Result<aws_sdk_bedrockruntime::Client> {
            let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
                .region(aws_config::Region::new(self.region.clone()))
                .load()
                .await;
            Ok(aws_sdk_bedrockruntime::Client::new(&config))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bedrock_model_id_format() {
        assert_eq!(
            normalize_bedrock_model("claude-3-sonnet"),
            "anthropic.claude-3-sonnet-20240229-v1:0"
        );
    }

    #[test]
    fn bedrock_model_id_already_qualified() {
        assert_eq!(
            normalize_bedrock_model("anthropic.claude-3-sonnet-20240229-v1:0"),
            "anthropic.claude-3-sonnet-20240229-v1:0"
        );
    }

    #[test]
    fn bedrock_provider_name() {
        assert_eq!(BedrockProvider::provider_name(), "bedrock");
    }
}
