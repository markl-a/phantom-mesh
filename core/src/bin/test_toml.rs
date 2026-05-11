use phantom_mesh::AgentsConfig;

fn main() {
    let content = r#"
[core]
host = "0.0.0.0"
port = 7878

[cluster]
node_name      = "rog-phone"
cluster_secret = "phantom-cluster-2026"
capabilities   = ["web_fetch", "search", "analysis", "mobile"]
peers = [
  "http://100.87.93.58:7878",
  "http://100.87.70.65:7879",
  "http://100.107.205.98:7878",
  "http://100.106.176.125:7878",
]

[providers.groq]
base_url      = "https://api.groq.com/openai/v1"
api_key       = "gsk_test"
default_model = "llama-3.3-70b-versatile"

[providers.gemini]
base_url      = "https://generativelanguage.googleapis.com/v1beta/openai"
api_key       = "AIzaTest"
default_model = "gemini-2.5-flash"

[agent.master]
provider = "groq"
model    = "llama-3.3-70b-versatile"
tools    = ["shell", "file_read", "file_write", "web_fetch", "content_search"]
instructions = "Mobile Android agent."
"#;
    match toml::from_str::<AgentsConfig>(content) {
        Ok(cfg) => {
            println!("Parse OK! providers: {:?}", cfg.providers.keys().collect::<Vec<_>>());
            for (name, entry) in &cfg.providers {
                println!("  {}: api_key={:?}, url={:?}", name, entry.api_key, entry.url);
            }
        }
        Err(e) => println!("Parse FAILED: {}", e),
    }
}
