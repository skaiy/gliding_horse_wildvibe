use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use parking_lot::RwLock;
use tokio::sync::Mutex;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::batch::emitter::BatchEventEmitter;
use crate::batch::error::BatchError;
use crate::batch::trigger::TriggerSystem;
use crate::batch::types::{
    BatchAgentConfig, BatchAgentStatus, BatchMetrics, ExtractionResult,
};
use crate::batch::window::WindowConfig;
use crate::batch::window::SlidingWindow;
use crate::core::event_bus::EventBus;
use crate::core::CoreError;

/// Holds the runtime state for a single Batch Agent instance.
pub struct BatchAgentInstance {
    pub name: String,
    pub config: BatchAgentConfig,
    pub status: BatchAgentStatus,
    pub window: Arc<RwLock<SlidingWindow>>,
    pub trigger_system: TriggerSystem,
    pub metrics: BatchMetrics,
}

pub struct BatchAgentManager {
    agents: HashMap<String, BatchAgentInstance>,
    event_bus: Option<Arc<EventBus>>,
    emitter: Option<BatchEventEmitter>,
    running: bool,
}

impl BatchAgentManager {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            event_bus: None,
            emitter: None,
            running: false,
        }
    }

    pub fn with_event_bus(mut self, event_bus: Arc<EventBus>) -> Self {
        let emitter = BatchEventEmitter::new(event_bus.clone());
        self.event_bus = Some(event_bus);
        self.emitter = Some(emitter);
        self
    }

    pub fn register(&mut self, config: BatchAgentConfig) -> Result<(), BatchError> {
        let name = config.name.clone();

        if self.agents.contains_key(&name) {
            return Err(BatchError::AgentAlreadyExists { name });
        }

        let window_config = match &config.window_type {
            crate::batch::types::WindowType::MessageCount(n) => WindowConfig {
                max_entries: *n,
                min_entries: 1,
                time_window_secs: u64::MAX,
                intent_shift_threshold: 0.6,
            },
            crate::batch::types::WindowType::TimeWindow(secs) => WindowConfig {
                max_entries: usize::MAX,
                min_entries: 1,
                time_window_secs: *secs,
                intent_shift_threshold: 0.6,
            },
            crate::batch::types::WindowType::Hybrid { max_messages, max_seconds } => WindowConfig {
                max_entries: *max_messages,
                min_entries: 1,
                time_window_secs: *max_seconds,
                intent_shift_threshold: 0.6,
            },
            crate::batch::types::WindowType::Manual => WindowConfig {
                max_entries: usize::MAX,
                min_entries: 0,
                time_window_secs: u64::MAX,
                intent_shift_threshold: 0.6,
            },
        };

        let window = Arc::new(RwLock::new(SlidingWindow::new(window_config)));
        let mut trigger_system = TriggerSystem::new(config.triggers.clone(), window.clone());

        // Auto-register event types
        if let Some(ref mut emitter) = self.emitter {
            emitter.set_agent_config(&name, config.emit_on.clone());
        }
        if let Some(ref event_bus) = self.event_bus {
            trigger_system.listen_to(event_bus, vec![]);
        }

        let instance = BatchAgentInstance {
            name: name.clone(),
            config: config.clone(),
            status: BatchAgentStatus::Registered,
            window,
            trigger_system,
            metrics: BatchMetrics::default(),
        };

        self.agents.insert(name.clone(), instance);
        info!(agent = %name, domain = %config.business_domain, "Batch Agent registered");
        Ok(())
    }

    pub fn unregister(&mut self, name: &str) -> Result<(), BatchError> {
        if !self.agents.contains_key(name) {
            return Err(BatchError::AgentNotFound { name: name.into() });
        }
        self.agents.remove(name);
        debug!(agent = %name, "Batch Agent unregistered");
        Ok(())
    }

    pub async fn start(&mut self, name: Option<&str>) -> Result<(), BatchError> {
        if let Some(n) = name {
            let instance = self.agents.get_mut(n).ok_or_else(|| {
                BatchError::AgentNotFound { name: n.into() }
            })?;
            if instance.status == BatchAgentStatus::Running {
                return Err(BatchError::AgentAlreadyRunning { name: n.into() });
            }
            instance.status = BatchAgentStatus::Running;
            if let Some(ref emitter) = self.emitter {
                emitter.emit_agent_started(n).await;
            }
            info!(agent = %n, "Batch Agent started");
        } else {
            let names: Vec<String> = self.agents.keys().cloned().collect();
            for n in names {
                if let Some(instance) = self.agents.get_mut(&n) {
                    if instance.status != BatchAgentStatus::Running {
                        instance.status = BatchAgentStatus::Running;
                        if let Some(ref emitter) = self.emitter {
                            emitter.emit_agent_started(&n).await;
                        }
                    }
                }
            }
            self.running = true;
            info!("All batch agents started");
        }
        Ok(())
    }

    pub async fn stop(&mut self, name: Option<&str>) -> Result<(), BatchError> {
        if let Some(n) = name {
            let instance = self.agents.get_mut(n).ok_or_else(|| {
                BatchError::AgentNotFound { name: n.into() }
            })?;
            instance.status = BatchAgentStatus::Stopped;
            if let Some(ref emitter) = self.emitter {
                emitter.emit_agent_stopped(n, "manual_stop").await;
            }
        } else {
            let names: Vec<String> = self.agents.keys().cloned().collect();
            for n in names {
                if let Some(instance) = self.agents.get_mut(&n) {
                    instance.status = BatchAgentStatus::Stopped;
                    if let Some(ref emitter) = self.emitter {
                        emitter.emit_agent_stopped(&n, "manual_stop").await;
                    }
                }
            }
            self.running = false;
        }
        Ok(())
    }

    pub fn get_status(&self, name: &str) -> Option<BatchAgentStatus> {
        self.agents.get(name).map(|a| a.status.clone())
    }

    pub fn get_window_status(&self, name: &str) -> Option<crate::batch::types::WindowStatus> {
        self.agents
            .get(name)
            .map(|a| a.window.read().status())
    }

    pub fn get_metrics(&self, name: &str) -> Option<BatchMetrics> {
        self.agents.get(name).map(|a| a.metrics.clone())
    }

    pub fn push_message(&mut self, agent_name: &str, entry: crate::batch::types::WindowEntry) -> Result<(), BatchError> {
        let instance = self.agents.get_mut(agent_name).ok_or_else(|| {
            BatchError::AgentNotFound { name: agent_name.into() }
        })?;
        instance.window.write().push(entry)
    }

    pub fn evaluate_triggers(&self, agent_name: &str) -> Vec<crate::batch::types::TriggerReason> {
        self.agents
            .get(agent_name)
            .map(|a| {
                let rt = tokio::runtime::Handle::try_current();
                match rt {
                    Ok(handle) => {
                        handle.block_on(async { a.trigger_system.evaluate().await })
                    }
                    Err(_) => vec![],
                }
            })
            .unwrap_or_default()
    }

    pub fn drain_window(&mut self, agent_name: &str) -> Option<Vec<crate::batch::types::WindowEntry>> {
        self.agents
            .get_mut(agent_name)
            .map(|a| a.window.write().drain())
    }

    pub fn list_agents(&self) -> Vec<&str> {
        self.agents.keys().map(|s| s.as_str()).collect()
    }

    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn emitter(&self) -> Option<&BatchEventEmitter> {
        self.emitter.as_ref()
    }
}

impl Default for BatchAgentManager {
    fn default() -> Self {
        Self::new()
    }
}
