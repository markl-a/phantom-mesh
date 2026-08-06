//! Spectyn Mesh Multi-Agent Coordination System
//! 
//! 實現多代理協作所需的協調者模式和工作者池

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ════════════════════════════════════════════════════════════════════════════════
// Enumerations
// ════════════════════════════════════════════════════════════════════════════════

/// 節點角色
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeRole {
    Coordinator,  // 協調者：負任務分發
    Worker,       // 工作執行緒：實際執行任務
}

/// 任務優先順序
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Priority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// 任務狀態
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,       // 待處理
    Assigned,     // 已分發
    Running,      // 執行中
    Completed,    // 完成
    Failed,       // 失敗
    Cancelled,    // 已取消
}

// ════════════════════════════════════════════════════════════════════════════════
// Data Structures
// ════════════════════════════════════════════════════════════════════════════════

/// 節點能力標籤
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeCapabilities {
    pub can_execute_shell: bool,
    pub can_file_ops: bool,
    pub can_git_ops: bool,
    pub can_web_fetch: bool,
    pub can_ml_training: bool,
    pub max_concurrent_tasks: usize,
    pub memory_mb: u64,
    pub cpu_cores: u32,
}

/// 工作節點資訊
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerNode {
    pub id: String,
    pub name: String,
    pub role: NodeRole,
    pub capabilities: NodeCapabilities,
    pub status: String,           // "online", "busy", "offline"
    pub current_tasks: Vec<Uuid>,
    pub last_heartbeat: i64,
}

/// 任務定義
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub description: String,
    pub payload: Value,             // 任務輸入資料
    pub required_capabilities: NodeCapabilities,
    pub priority: Priority,
    pub status: TaskStatus,
    pub assigned_to: Option<String>,
    pub result: Option<Value>,       // 任務輸出結果
    pub error: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
}

impl Task {
    pub fn new(description: String, payload: Value, required_capabilities: NodeCapabilities, priority: Priority) -> Self {
        Self {
            id: Uuid::new_v4(),
            description,
            payload,
            required_capabilities,
            priority,
            status: TaskStatus::Pending,
            assigned_to: None,
            result: None,
            error: None,
            created_at:chrono::Utc::now().timestamp(),
            started_at: None,
            completed_at: None,
        }
    }
}

/// 任務佇列
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskQueue {
    pub pending: Vec<Uuid>,
    pub assigned: Vec<Uuid>,
    pub running: Vec<Uuid>,
    pub completed: Vec<Uuid>,
    pub failed: Vec<Uuid>,
}

// ════════════════════════════════════════════════════════════════════════════════
// Multi-Agent Coordinator
// ════════════════════════════════════════════════════════════════════════════════

/// 協調者管理器
pub struct Coordinator {
    nodes: Arc<RwLock<HashMap<String, WorkerNode>>>,
    tasks: Arc<RwLock<HashMap<Uuid, Task>>>,
    task_queue: Arc<RwLock<TaskQueue>>,
}

impl Coordinator {
    pub fn new() -> Self {
        Self {
            nodes: Arc::new(RwLock::new(HashMap::new())),
            tasks: Arc::new(RwLock::new(HashMap::new())),
            task_queue: Arc::new(RwLock::new(TaskQueue::default())),
        }
    }

    /// 註冊新節點
    pub async fn register_node(&self, node: WorkerNode) -> Result<(), String> {
        let mut nodes = self.nodes.write().await;
        
        // 檢查是否已存在
        if nodes.contains_key(&node.id) {
            return Err(format!("Node {} already registered", node.id));
        }
        
        nodes.insert(node.id.clone(), node);
        Ok(())
    }

    /// 註銷節點
    pub async fn unregister_node(&self, node_id: &str) -> Result<(), String> {
        let mut nodes = self.nodes.write().await;
        nodes.remove(node_id).ok_or_else(|| format!("Node {} not found", node_id))?;
        Ok(())
    }

    /// 提交新任務
    pub async fn submit_task(&self, task: Task) -> Result<Uuid, String> {
        let task_id = task.id;
        
        // 寫入任務
        {
            let mut tasks = self.tasks.write().await;
            tasks.insert(task_id, task);
        }
        
        // 加入待處理佇列
        {
            let mut queue = self.task_queue.write().await;
            queue.pending.push(task_id);
        }
        
        Ok(task_id)
    }

    /// 獲取最佳工作節點（基於能力匹配）
    async fn find_best_worker(&self, required: &NodeCapabilities) -> Option<WorkerNode> {
        let nodes = self.nodes.read().await;
        
        let mut candidates: Vec<_> = nodes.values()
            .filter(|n| n.role == NodeRole::Worker && n.status == "online")
            .filter(|n| self.matches_capabilities(&n.capabilities, required))
            .filter(|n| n.current_tasks.len() < n.capabilities.max_concurrent_tasks)
            .cloned()
            .collect();
        
        // 選擇負載最低的節點
        candidates.sort_by_key(|n| n.current_tasks.len());
        
        candidates.into_iter().next()
    }

    /// 能力匹配檢查
    fn matches_capabilities(&self, worker: &NodeCapabilities, required: &NodeCapabilities) -> bool {
        !worker.can_execute_shell && required.can_execute_shell ||
        !worker.can_file_ops && required.can_file_ops ||
        !worker.can_git_ops && required.can_git_ops ||
        !worker.can_web_fetch && required.can_web_fetch
    }

    /// 分發任務到工作節點
    pub async fn dispatch_tasks(&self) -> Result<(), String> {
        loop {
            let task_id = {
                let mut queue = self.task_queue.write().await;
                if queue.pending.is_empty() {
                    break;
                }
                queue.pending.remove(0)
            };
            
            let task = {
                let tasks = self.tasks.read().await;
                tasks.get(&task_id).cloned()
            };
            
            let Some(task) = task else { continue; };
            
            // 找最佳工作節點
            let worker = self.find_best_worker(&task.required_capabilities).await;
            
            if let Some(worker) = worker {
                let mut tasks = self.tasks.write().await;
                if let Some(t) = tasks.get_mut(&task_id) {
                    t.status = TaskStatus::Assigned;
                    t.assigned_to = Some(worker.id.clone());
                    t.started_at = Some(chrono::Utc::now().timestamp());
                }
                
                drop(tasks);
                
                // 更新佇列
                let mut queue = self.task_queue.write().await;
                queue.pending.retain(|&id| id != task_id);
                queue.assigned.push(task_id);
                
                // 更新節點任務
                let mut nodes = self.nodes.write().await;
                if let Some(n) = nodes.get_mut(&worker.id) {
                    n.current_tasks.push(task_id);
                    n.status = "busy";
                }
            }
        }
        
        Ok(())
    }

    /// 獲取任務狀態
    pub async fn get_task_status(&self, task_id: Uuid) -> Option<TaskStatus> {
        let tasks = self.tasks.read().await;
        tasks.get(&task_id).map(|t| t.status)
    }

    /// 標記任務完成
    pub async fn complete_task(&self, task_id: Uuid, result: Value) -> Result<(), String> {
        let (node_id, worker_id) = {
            let mut tasks = self.tasks.write().await;
            let task = tasks.get_mut(&task_id).ok_or("Task not found")?;
            task.status = TaskStatus::Completed;
            task.result = Some(result);
            task.completed_at = Some(chrono::Utc::now().timestamp());
            (task.assigned_to.clone(), task.id)
        };
        
        // 更新佇列
        {
            let mut queue = self.task_queue.write().await;
            queue.assigned.retain(|&id| id != task_id);
            queue.completed.push(task_id);
        }
        
        // 釋放工作節點
        if let Some(node_id) = node_id {
            let mut nodes = self.nodes.write().await;
            if let Some(n) = nodes.get_mut(&node_id) {
                n.current_tasks.retain(|&id| id != task_id);
                n.status = "online";
            }
        }
        
        Ok(())
    }

    /// 獲取所有節點
    pub async fn list_nodes(&self) -> Vec<WorkerNode> {
        let nodes = self.nodes.read().await;
        nodes.values().cloned().collect()
    }

    /// 獲取任務列表
    pub async fn list_tasks(&self, status: Option<TaskStatus>) -> Vec<Task> {
        let tasks = self.tasks.read().await;
        
        match status {
            Some(s) => tasks.values().filter(|t| t.status == s).cloned().collect(),
            None => tasks.values().cloned().collect(),
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════════
// RPC Interface for Worker Communication
// ════════════════════════════════════════════════════════════════════════════════

/// RPC 任務請求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcTaskRequest {
    pub task_id: Uuid,
    pub description: String,
    pub payload: Value,
    pub capabilities: NodeCapabilities,
}

/// RPC 回應
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcTaskResponse {
    pub task_id: Uuid,
    pub status: TaskStatus,
    pub result: Option<Value>,
    pub error: Option<String>,
}

// ════════════════════════════════════════════════════════════════════════════════
// Example Usage
// ════════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_coordinator() {
        let coord = Coordinator::new();
        
        // 註冊工作節點
        coord.register_node(WorkerNode {
            id: "worker-1".into(),
            name: "Worker One".into(),
            role: NodeRole::Worker,
            capabilities: NodeCapabilities {
                can_execute_shell: true,
                can_file_ops: true,
                can_git_ops: true,
                can_web_fetch: true,
                can_ml_training: false,
                max_concurrent_tasks: 2,
                memory_mb: 8192,
                cpu_cores: 4,
            },
            status: "online".into(),
            current_tasks: vec![],
            last_heartbeat: 0,
        }).await.unwrap();
        
        // 提交任務
        let task = Task::new(
            "Run tests".into(),
            serde_json::json!({"command": "cargo test"}),
            NodeCapabilities {
                can_execute_shell: true,
                ..Default::default()
            },
            Priority::High,
        );
        
        let task_id = coord.submit_task(task).await.unwrap();
        
        // 分發任務
        coord.dispatch_tasks().await.unwrap();
        
        // 檢查狀態
        let status = coord.get_task_status(task_id).await;
        assert_eq!(status, Some(TaskStatus::Assigned));
        
        // 完成任務
        coord.complete_task(task_id, serde_json::json!({"output": "All tests passed"})).await.unwrap();
        
        let status = coord.get_task_status(task_id).await;
        assert_eq!(status, Some(TaskStatus::Completed));
    }
}