//! A2A (Agent-to-Agent) Protocol — Google's agent interoperability standard.
//! Serves `/.well-known/agent.json` and handles `POST /a2a/tasks`.
//!
//! Reference: OpenCrust's A2A implementation.

use serde::{Deserialize, Serialize};

/// Agent Card — served at `/.well-known/agent.json`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    pub url: String,
    pub version: String,
    #[serde(default)]
    pub capabilities: AgentCapabilities,
    #[serde(default)]
    pub skills: Vec<AgentSkill>,
}

/// Agent capabilities declaration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentCapabilities {
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub push_notifications: bool,
    #[serde(default)]
    pub state_transition_history: bool,
}

/// A skill the agent can perform
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A2A Task — created via POST /a2a/tasks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ATask {
    pub id: String,
    pub status: TaskStatus,
    pub input: String,
    #[serde(default)]
    pub output: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Task status following A2A spec
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Submitted,
    Working,
    Completed,
    Failed,
    Cancelled,
}

/// Request to create a new A2A task
#[derive(Debug, Clone, Deserialize)]
pub struct CreateTaskRequest {
    pub input: String,
    #[serde(default)]
    pub agent: Option<String>,
}

impl AgentCard {
    /// Build the default Clawtex agent card
    pub fn clawtex_default(base_url: &str) -> Self {
        Self {
            name: "Clawtex".to_string(),
            description: "Autonomous AI agent daemon with tool execution, multi-provider LLM routing, and workflow automation.".to_string(),
            url: base_url.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            capabilities: AgentCapabilities {
                streaming: true,
                push_notifications: false,
                state_transition_history: true,
            },
            skills: vec![
                AgentSkill {
                    id: "general".to_string(),
                    name: "General Assistant".to_string(),
                    description: "Answer questions, execute tasks, use tools".to_string(),
                    tags: vec!["general".to_string(), "tools".to_string()],
                },
                AgentSkill {
                    id: "code".to_string(),
                    name: "Code Generation".to_string(),
                    description: "Write and execute code, manage files".to_string(),
                    tags: vec!["code".to_string(), "development".to_string()],
                },
                AgentSkill {
                    id: "research".to_string(),
                    name: "Web Research".to_string(),
                    description: "Search the web, analyze content".to_string(),
                    tags: vec!["research".to_string(), "web".to_string()],
                },
            ],
        }
    }
}

impl A2ATask {
    pub fn new(input: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            status: TaskStatus::Submitted,
            input: input.to_string(),
            output: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: None,
        }
    }

    pub fn set_working(&mut self) {
        self.status = TaskStatus::Working;
        self.updated_at = Some(chrono::Utc::now().to_rfc3339());
    }

    pub fn set_completed(&mut self, output: &str) {
        self.status = TaskStatus::Completed;
        self.output = Some(output.to_string());
        self.updated_at = Some(chrono::Utc::now().to_rfc3339());
    }

    pub fn set_failed(&mut self, error: &str) {
        self.status = TaskStatus::Failed;
        self.output = Some(format!("Error: {}", error));
        self.updated_at = Some(chrono::Utc::now().to_rfc3339());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_card_default() {
        let card = AgentCard::clawtex_default("http://localhost:7878");
        assert_eq!(card.name, "Clawtex");
        assert!(card.capabilities.streaming);
        assert_eq!(card.skills.len(), 3);
        assert_eq!(card.url, "http://localhost:7878");
    }

    #[test]
    fn test_agent_card_serde() {
        let card = AgentCard::clawtex_default("http://localhost:7878");
        let json = serde_json::to_string_pretty(&card).unwrap();
        assert!(json.contains("Clawtex"));
        let parsed: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "Clawtex");
    }

    #[test]
    fn test_a2a_task_lifecycle() {
        let mut task = A2ATask::new("Hello, please help");
        assert_eq!(task.status, TaskStatus::Submitted);
        assert!(task.output.is_none());

        task.set_working();
        assert_eq!(task.status, TaskStatus::Working);
        assert!(task.updated_at.is_some());

        task.set_completed("Here is my response");
        assert_eq!(task.status, TaskStatus::Completed);
        assert_eq!(task.output.as_deref(), Some("Here is my response"));
    }

    #[test]
    fn test_a2a_task_failure() {
        let mut task = A2ATask::new("Do something impossible");
        task.set_failed("No model available");
        assert_eq!(task.status, TaskStatus::Failed);
        assert!(task.output.as_ref().unwrap().contains("Error"));
    }

    #[test]
    fn test_task_status_serde() {
        let status = TaskStatus::Working;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"working\"");
        let parsed: TaskStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, TaskStatus::Working);
    }

    #[test]
    fn test_create_task_request() {
        let json = r#"{"input": "Help me write code", "agent": "coder"}"#;
        let req: CreateTaskRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.input, "Help me write code");
        assert_eq!(req.agent.as_deref(), Some("coder"));
    }
}
