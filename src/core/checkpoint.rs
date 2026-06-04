use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::memory::l0_store::L0Store;
use crate::CoreError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointData {
    pub checkpoint_iri: String,
    pub task_iri: String,
    pub name: String,
    pub node_count: i32,
    pub total_size_bytes: i32,
    pub created_at: DateTime<Utc>,
    pub tags: Vec<String>,
    pub nodes_json: String,
    pub session_messages_json: String,
    pub agent_state_json: String,
}

pub struct CheckpointManager {
    l0: Option<Arc<L0Store>>,
    task_checkpoints: RwLock<HashMap<String, Vec<String>>>,
    counter: AtomicU64,
}

impl CheckpointManager {
    pub fn new() -> Self {
        Self {
            l0: None,
            task_checkpoints: RwLock::new(HashMap::new()),
            counter: AtomicU64::new(0),
        }
    }

    pub fn with_persistence(l0: Arc<L0Store>) -> Self {
        Self {
            l0: Some(l0),
            task_checkpoints: RwLock::new(HashMap::new()),
            counter: AtomicU64::new(0),
        }
    }

    pub fn create(
        &self,
        task_iri: &str,
        name: &str,
        nodes_json: &str,
        session_messages_json: &str,
        agent_state_json: &str,
        tags: &[String],
    ) -> Result<CheckpointData, CoreError> {
        let seq = self.counter.fetch_add(1, Ordering::SeqCst);
        let checkpoint_iri = format!(
            "iri://checkpoint/{}/seq_{}",
            task_iri.strip_prefix("iri://").unwrap_or(task_iri),
            seq
        );

        let nodes: Vec<serde_json::Value> =
            serde_json::from_str(nodes_json).unwrap_or_default();
        let node_count = nodes.len() as i32;
        let total_size_bytes =
            nodes_json.len() as i32 + session_messages_json.len() as i32 + agent_state_json.len() as i32;

        let checkpoint = CheckpointData {
            checkpoint_iri: checkpoint_iri.clone(),
            task_iri: task_iri.to_string(),
            name: name.to_string(),
            node_count,
            total_size_bytes,
            created_at: Utc::now(),
            tags: tags.to_vec(),
            nodes_json: nodes_json.to_string(),
            session_messages_json: session_messages_json.to_string(),
            agent_state_json: agent_state_json.to_string(),
        };

        if let Some(ref l0) = self.l0 {
            let content = serde_json::to_string(&checkpoint).map_err(|e| CoreError::Internal {
                message: format!("Failed to serialize checkpoint: {}", e),
            })?;
            l0.store(&checkpoint_iri, &content)?;
        }

        {
            let mut task_cps = self.task_checkpoints.write();
            task_cps
                .entry(task_iri.to_string())
                .or_insert_with(Vec::new)
                .push(checkpoint_iri.clone());
        }

        Ok(checkpoint)
    }

    pub fn restore(&self, checkpoint_iri: &str) -> Result<CheckpointData, CoreError> {
        if let Some(ref l0) = self.l0 {
            if let Ok(Some(entry)) = l0.retrieve(checkpoint_iri) {
                return serde_json::from_str(&entry.content).map_err(|e| CoreError::Internal {
                    message: format!("Invalid checkpoint data: {}", e),
                });
            }
        }
        Err(CoreError::Internal {
            message: format!("Checkpoint not found: {}", checkpoint_iri),
        })
    }

    pub fn restore_latest(&self, task_iri: &str) -> Result<Option<CheckpointData>, CoreError> {
        let list = self.list(task_iri, 1);
        Ok(list.into_iter().next())
    }

    pub fn list(&self, task_iri: &str, limit: i32) -> Vec<CheckpointData> {
        // 先尝试内存索引（同进程内有效）
        {
            let task_cps = self.task_checkpoints.read();
            if let Some(cp_iris) = task_cps.get(task_iri) {
                let mut results: Vec<CheckpointData> = cp_iris
                    .iter()
                    .rev()
                    .filter_map(|iri| {
                        if let Some(ref l0) = self.l0 {
                            l0.retrieve(iri)
                                .ok()
                                .flatten()
                                .and_then(|e| serde_json::from_str(&e.content).ok())
                        } else {
                            None
                        }
                    })
                    .collect();
                results.truncate(limit as usize);
                return results;
            }
        }
        // 内存索引未命中 → 从 L0 按 IRI 前缀扫描（跨进程恢复用）
        if let Some(ref l0) = self.l0 {
            let stripped = task_iri.strip_prefix("iri://").unwrap_or(task_iri);
            let prefix = format!("iri://checkpoint/{}/", stripped);
            if let Ok(entries) = l0.scan_iri_prefix(&prefix, 100) {
                let mut results: Vec<CheckpointData> = entries
                    .iter()
                    .filter_map(|e| serde_json::from_str(&e.content).ok())
                    .collect();
                results.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                results.truncate(limit as usize);
                return results;
            }
        }
        Vec::new()
    }

    pub fn delete(&self, checkpoint_iri: &str) -> Result<(), CoreError> {
        if let Some(ref l0) = self.l0 {
            if l0.retrieve(checkpoint_iri)?.is_none() {
                return Err(CoreError::Internal {
                    message: format!("Checkpoint not found: {}", checkpoint_iri),
                });
            }
            l0.delete(checkpoint_iri)?;
        }
        {
            let mut task_cps = self.task_checkpoints.write();
            for iris in task_cps.values_mut() {
                iris.retain(|iri| iri != checkpoint_iri);
            }
        }
        Ok(())
    }

    pub fn checkpoint_count(&self) -> u64 {
        self.task_checkpoints.read().values().map(|v| v.len() as u64).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_in_memory() {
        let manager = CheckpointManager::new();
        let checkpoint = manager
            .create(
                "iri://task/123",
                "test",
                r#"[{"@id":"iri://node/1"}]"#,
                r#"[{"role":"user"}]"#,
                r#"{"status":"running"}"#,
                &["important".to_string()],
            )
            .unwrap();
        assert!(checkpoint.checkpoint_iri.starts_with("iri://checkpoint/"));
        assert_eq!(checkpoint.task_iri, "iri://task/123");
    }

    #[test]
    fn test_list_empty() {
        let manager = CheckpointManager::new();
        let list = manager.list("iri://task/nonexistent", 10);
        assert!(list.is_empty());
    }

    #[test]
    fn test_list_via_l0_scan_cross_process() {
        use std::sync::Arc;
        use crate::memory::l0_store::L0Store;

        let dir = tempfile::TempDir::new().unwrap();
        let l0 = Arc::new(L0Store::new(dir.path().to_str().unwrap()).unwrap());
        let mgr = CheckpointManager::with_persistence(l0.clone());

        // 创建检查点（模拟在上一次进程中运行）
        mgr.create(
            "iri://task/abc-123",
            "finish_DA",
            "[]",
            r#"[{"role":"user","content":"hello"}]"#,
            r#"{"turn":3}"#,
            &["DA".to_string()],
        ).unwrap();

        // 新建 CheckpointManager（模拟跨进程：新实例、空内存索引）
        let mgr2 = CheckpointManager::with_persistence(l0.clone());

        // restore_latest 必须找到检查点（通过 scan_iri_prefix 回退）
        let cp = mgr2.restore_latest("iri://task/abc-123").unwrap();
        assert!(cp.is_some(), "跨进程恢复必须找到检查点");
        assert_eq!(cp.unwrap().task_iri, "iri://task/abc-123");

        // list 也必须能找到
        let list = mgr2.list("iri://task/abc-123", 10);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "finish_DA");
    }
}
