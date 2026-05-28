use std::sync::Arc;

use agent_os::config::AgentSettings;
use agent_os::core::agent_runner::TaskResult;
use agent_os::core::event_bus::{EventBus, Event};
use agent_os::core::sa::SupervisorAgent;
use agent_os::gateway::UnifiedGateway;
use agent_os::memory::l0_store::L0Store;
use agent_os::memory::l2_blackboard::Blackboard;
use agent_os::memory::l3_projection::ProjectionEngine;
use agent_os::memory::memory_manager::MemoryManager;
use agent_os::templates::template_engine::TemplateEngine;
use agent_os::tools::skill_registry::SkillRegistry;
use agent_os::CoreConfig;
use tempfile::TempDir;
use tokio::sync::broadcast;
use tracing::info;

use crate::config::CliConfig;

#[derive(Debug, Clone)]
pub struct AgentEvent {
    pub event_type: String,
    pub source: String,
    pub payload: String,
}

pub struct CodeCliEngine {
    sa: SupervisorAgent,
    event_bus: Arc<EventBus>,
    config: CliConfig,
    _temp_dir: TempDir,
}

impl CodeCliEngine {
    pub fn new(config: CliConfig) -> anyhow::Result<Self> {
        // Set the process working directory to the configured workspace so that
        // agent_os tool handlers (execute_file_read/write/edit, execute_bash, …)
        // resolve relative paths against the correct root. Without this they
        // default to std::env::current_dir() which may be anything.
        let workspace_abs = std::path::Path::new(&config.workspace)
            .canonicalize()
            .unwrap_or_else(|_| std::path::PathBuf::from(&config.workspace));
        std::env::set_current_dir(&workspace_abs)
            .map_err(|e| anyhow::anyhow!("无法切换到工作目录 '{}': {}", workspace_abs.display(), e))?;

        let gateway = Arc::new(UnifiedGateway::new(&config.gateway)?);
        let dir = tempfile::TempDir::new()?;

        let l0 = Arc::new(
            L0Store::new(dir.path().join("l0").to_string_lossy().as_ref())
                .map_err(|e| anyhow::anyhow!("L0Store 创建失败: {}", e))?,
        );
        let l2 = Arc::new(
            Blackboard::new()
                .map_err(|e| anyhow::anyhow!("Blackboard 创建失败: {}", e))?,
        );
        let proj = Arc::new(ProjectionEngine::new(l2.clone(), 500));
        let core_config = CoreConfig::default();
        let mm = Arc::new(tokio::sync::Mutex::new(MemoryManager::new(
            l0.clone(),
            l2.clone(),
            proj.clone(),
            core_config,
        )));

        let templates_dir = dir.path().join("templates");
        std::fs::create_dir_all(&templates_dir)?;
        let tmpl = Arc::new(
            TemplateEngine::new(&templates_dir)
                .map_err(|e| anyhow::anyhow!("TemplateEngine 创建失败: {}", e))?,
        );

        let skills = Arc::new(SkillRegistry::new());
        let agent_settings = AgentSettings::default();

        let runner = Arc::new(agent_os::core::agent_runner::AgentRunner::new(
            gateway,
            skills.clone(),
            l2.clone(),
            l0,
            mm,
            tmpl.clone(),
            agent_settings,
        ));

        let event_bus = Arc::new(EventBus::new(100));

        let sa = SupervisorAgent::new(
            runner,
            tmpl,
            skills,
            event_bus.clone(),
            config.max_iterations,
        )
        .with_memory(Some(l2), None, None);

        info!(
            model = %config.model,
            workspace = %config.workspace,
            max_iterations = config.max_iterations,
            "Code CLI 引擎初始化完成"
        );

        Ok(Self {
            sa,
            event_bus,
            config,
            _temp_dir: dir,
        })
    }

    pub fn rebuild(&mut self) -> anyhow::Result<()> {
        *self = Self::new(self.config.clone())?;
        Ok(())
    }

    pub fn rebuild_with_model(&mut self, model: String) -> anyhow::Result<()> {
        self.config = self.config.clone_with_model(model);
        *self = Self::new(self.config.clone())?;
        Ok(())
    }

    pub fn model(&self) -> &str {
        &self.config.model
    }

    pub fn workspace(&self) -> &str {
        &self.config.workspace
    }

    pub fn max_iterations(&self) -> u32 {
        self.config.max_iterations
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.event_bus.subscribe()
    }

    pub async fn process_task(&mut self, user_input: &str) -> anyhow::Result<(String, TaskResult)> {
        let task_id = uuid::Uuid::new_v4().to_string();
        let task_iri = format!("iri://task/{}", task_id);

        let result = self.sa.process_task(user_input, &task_iri).await?;

        info!(
            task_iri = %task_iri,
            status = %result.status,
            turn_count = result.turn_count,
            tool_call_count = result.tool_call_count,
            "任务处理完成"
        );

        Ok((task_iri, result))
    }

    /// Returns a clone of the internal EventBus (for supplementary input / event monitoring).
    pub fn event_bus(&self) -> Arc<EventBus> {
        self.event_bus.clone()
    }

    /// Process a task with an externally-generated task IRI so the caller
    /// can emit supplementary input events during execution.
    pub async fn process_task_with_iri(&mut self, user_input: &str, task_iri: &str) -> anyhow::Result<TaskResult> {
        let result = self.sa.process_task(user_input, task_iri).await?;

        info!(
            task_iri = %task_iri,
            status = %result.status,
            turn_count = result.turn_count,
            tool_call_count = result.tool_call_count,
            "任务处理完成"
        );

        Ok(result)
    }
}
