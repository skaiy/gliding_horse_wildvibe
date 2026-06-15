use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::{json, Value};
use tracing::{debug, info, instrument, warn};

use crate::core::agent_instance::{AgentInstance, AgentRole, AgentStatus};
use crate::core::execution_event::{ExecutionEvent, ExecutionEventKind};
use crate::core::system_prompt::{SystemPromptBuilder, SystemPromptRegion, build_constitution_prompt};
use crate::gateway::unified_gateway::ChatMessage;
use crate::jsonld::{JsonLdContext, JsonLdNode};
use crate::memory::l1_session::L1Session;
use crate::methodology::integration::MethodologyPromptInjector;
use crate::tools::hooks::{HookContext, HookPoint, HookResult};
use crate::tools::tool_executor::ToolExecutor;
use crate::CoreError;

use super::{TaskContext, TaskResult, LLM_RESPONSE_FORMAT_NO_THOUGHT, LLM_RESPONSE_FORMAT_WITH_THOUGHT};

impl super::AgentRunner {
    pub async fn execute(
        &self,
        agent: &mut AgentInstance,
        ctx: TaskContext,
    ) -> Result<TaskResult, CoreError> {
        // AgentInit hook
        {
            let mut hook_ctx = HookContext::new(
                HookPoint::AgentInit,
                &agent.agent_id,
                &agent.role.to_string(),
            )
            .with_task(&ctx.task_iri, &ctx.task_iri);
            self.hook_manager
                .execute(HookPoint::AgentInit, &mut hook_ctx)
                .await;
        }

        agent.status = AgentStatus::Running;

        // TaskStart hook
        {
            let mut hook_ctx = HookContext::new(
                HookPoint::TaskStart,
                &agent.agent_id,
                &agent.role.to_string(),
            )
            .with_task(&ctx.task_iri, &ctx.task_iri);
            let hook_result = self
                .hook_manager
                .execute(HookPoint::TaskStart, &mut hook_ctx)
                .await;
            if hook_result == HookResult::Abort {
                agent.status = AgentStatus::Failed;
                return Ok(TaskResult {
                    task_iri: ctx.task_iri,
                    status: "aborted".to_string(),
                    summary: "Task aborted by hook".to_string(),
                    output: None,
                    jsonld_output: None,
                    artifacts: Vec::new(),
                    errors: vec!["Task aborted by hook".to_string()],
                    turn_count: 0,
                    tool_call_count: 0,
                    five_w2h_updates: None,
                    tracked_actions: Vec::new(),
                    archive_iri: None,
                });
            }
        }

        // AgentStart hook
        let mut hook_ctx = HookContext::new(
            HookPoint::AgentStart,
            &agent.agent_id,
            &agent.role.to_string(),
        )
        .with_task(&ctx.task_iri, &ctx.task_iri);
        let hook_result = self
            .hook_manager
            .execute(HookPoint::AgentStart, &mut hook_ctx)
            .await;
        if hook_result == HookResult::Abort {
            agent.status = AgentStatus::Failed;
            return Ok(TaskResult {
                task_iri: ctx.task_iri,
                status: "aborted".to_string(),
                summary: "Agent aborted by hook".to_string(),
                output: None,
                jsonld_output: None,
                artifacts: Vec::new(),
                errors: vec!["Agent aborted by hook".to_string()],
                turn_count: 0,
                tool_call_count: 0,
                five_w2h_updates: None,
                tracked_actions: Vec::new(),
                archive_iri: None,
            });
        }

        let mut session = self.memory_manager.lock().await.create_session(
            &agent.agent_id,
            &agent.role.to_string(),
            &ctx.task_iri,
        );

        // 计算任务 embedding，用于语义相关度淘汰
        if let Some(ref embedder) = self.embedder {
            if let Ok(task_emb) = embedder.embed(&ctx.objective).await {
                session.set_task_embedding(task_emb.clone());
                if let Some(ref tracker_lock) = self.relevance_tracker {
                    let mut tracker = tracker_lock.lock().unwrap();
                    tracker.reset();
                    tracker.set_task_context(task_emb);
                }
            }
        }

        // MemoryWrite hook for session creation
        {
            let mut hook_ctx = HookContext::new(
                HookPoint::MemoryWrite,
                &agent.agent_id,
                &agent.role.to_string(),
            )
            .with_task(&ctx.task_iri, &ctx.task_iri);
            self.hook_manager
                .execute(HookPoint::MemoryWrite, &mut hook_ctx)
                .await;
        }

        let result = self.exec(agent, ctx.clone(), &mut session).await;

        {
            let mut mm = self.memory_manager.lock().await;
            if !result.as_ref().map(|r| r.tracked_actions.is_empty()).unwrap_or(true) {
                if let Ok(ref r) = result {
                    let _ = mm.archive_session_actions(&r.task_iri, &r.tracked_actions, &r.summary);
                    if !r.tracked_actions.is_empty() {
                        let success_rate = r.tracked_actions.iter()
                            .filter(|a| a.status == crate::core::tracked_action::ActionStatus::Success).count() as f32
                            / r.tracked_actions.len().max(1) as f32;
                        let _ = mm.archive_experience(&r.task_iri, &agent.role.to_string(), &r.summary, success_rate);
                    }
                }
            }
            let _ = mm.finalize_session(session, &ctx.task_iri);
        }

        // TaskEnd hook
        {
            let mut hook_ctx =
                HookContext::new(HookPoint::TaskEnd, &agent.agent_id, &agent.role.to_string())
                    .with_task(&ctx.task_iri, &ctx.task_iri);
            self.hook_manager
                .execute(HookPoint::TaskEnd, &mut hook_ctx)
                .await;
        }

        // AgentEnd hook
        let mut hook_ctx = HookContext::new(
            HookPoint::AgentEnd,
            &agent.agent_id,
            &agent.role.to_string(),
        );
        self.hook_manager
            .execute(HookPoint::AgentEnd, &mut hook_ctx)
            .await;

        // Handle errors
        if let Ok(ref r) = result {
            if r.status == "failed" {
                let mut hook_ctx = HookContext::new(
                    HookPoint::AgentError,
                    &agent.agent_id,
                    &agent.role.to_string(),
                )
                .with_task(&ctx.task_iri, &ctx.task_iri);
                hook_ctx.error = Some(r.summary.clone());
                self.hook_manager
                    .execute(HookPoint::AgentError, &mut hook_ctx)
                    .await;

                let mut hook_ctx = HookContext::new(
                    HookPoint::TaskError,
                    &agent.agent_id,
                    &agent.role.to_string(),
                )
                .with_task(&ctx.task_iri, &ctx.task_iri);
                hook_ctx.error = Some(r.summary.clone());
                self.hook_manager
                    .execute(HookPoint::TaskError, &mut hook_ctx)
                    .await;
            }
        }

        result
    }

    /// 使用独立的 BizAgent 实例执行任务（Agent 隔离）
    pub async fn execute_with_biz_agent(
        &self,
        agent: &AgentInstance,
        ctx: TaskContext,
        plan_step: Option<crate::core::sa::PlanStep>,
    ) -> Result<TaskResult, CoreError> {
        use crate::core::biz_agent::{AgentConfig, BizAgent};

        // AgentInit hook
        {
            let mut hook_ctx = HookContext::new(
                HookPoint::AgentInit,
                &agent.agent_id,
                &agent.role.to_string(),
            )
            .with_task(&ctx.task_iri, &ctx.task_iri);
            self.hook_manager
                .execute(HookPoint::AgentInit, &mut hook_ctx)
                .await;
        }

        // 构建独立的 agent.md
        let agent_md = if let Some(ref step) = plan_step {
            self.build_agent_md_from_step(agent.role, step)
        } else {
            let context_data = self.gather_context_data_async(agent.role, &ctx).await;
            let model = self
                .gateway
                .get_model(&agent.role.to_string().to_lowercase());
            self.build_agent_md(agent.role, &ctx.objective, &context_data, &model)
        };

        // 创建 BizAgent 配置
        let config = AgentConfig {
            orchestrator_mode: false,
            max_sub_agents: 5,
            max_iterations: ctx.max_iterations,
            parallel_sub_agents: true,
        };

        // 创建独立的 BizAgent 实例
        let mut biz_agent = BizAgent::new(
            agent.agent_id.clone(),
            agent.role,
            &agent_md,
            Arc::new((*self).clone()),
            config,
        );

        // TaskStart hook
        {
            let mut hook_ctx = HookContext::new(
                HookPoint::TaskStart,
                &agent.agent_id,
                &agent.role.to_string(),
            )
            .with_task(&ctx.task_iri, &ctx.task_iri);
            let hook_result = self
                .hook_manager
                .execute(HookPoint::TaskStart, &mut hook_ctx)
                .await;
            if hook_result == HookResult::Abort {
            return Ok(TaskResult {
                task_iri: ctx.task_iri,
                status: "aborted".to_string(),
                summary: "Agent aborted by hook".to_string(),
                output: None,
                jsonld_output: None,
                artifacts: Vec::new(),
                errors: vec!["Agent aborted by hook".to_string()],
                turn_count: 0,
                tool_call_count: 0,
                five_w2h_updates: None,
                tracked_actions: Vec::new(),
                archive_iri: None,
            });
            }
        }

        // AgentStart hook
        let mut hook_ctx = HookContext::new(
            HookPoint::AgentStart,
            &agent.agent_id,
            &agent.role.to_string(),
        )
        .with_task(&ctx.task_iri, &ctx.task_iri);
        let hook_result = self
            .hook_manager
            .execute(HookPoint::AgentStart, &mut hook_ctx)
            .await;
        if hook_result == HookResult::Abort {
            return Ok(TaskResult {
                task_iri: ctx.task_iri,
                status: "aborted".to_string(),
                summary: "Agent aborted by hook".to_string(),
                output: None,
                jsonld_output: None,
                artifacts: Vec::new(),
                errors: vec!["Agent aborted by hook".to_string()],
                turn_count: 0,
                tool_call_count: 0,
                five_w2h_updates: None,
                tracked_actions: Vec::new(),
                archive_iri: None,
            });
        }

        // 执行 BizAgent（隔离环境）
        let result = biz_agent.execute(ctx.clone()).await;

        // TaskEnd hook
        {
            let mut hook_ctx =
                HookContext::new(HookPoint::TaskEnd, &agent.agent_id, &agent.role.to_string())
                    .with_task(&ctx.task_iri, &ctx.task_iri);
            self.hook_manager
                .execute(HookPoint::TaskEnd, &mut hook_ctx)
                .await;
        }

        // AgentEnd hook
        let mut hook_ctx = HookContext::new(
            HookPoint::AgentEnd,
            &agent.agent_id,
            &agent.role.to_string(),
        );
        self.hook_manager
            .execute(HookPoint::AgentEnd, &mut hook_ctx)
            .await;

        // Handle errors
        if result.status == "failed" {
            let mut hook_ctx = HookContext::new(
                HookPoint::AgentError,
                &agent.agent_id,
                &agent.role.to_string(),
            )
            .with_task(&ctx.task_iri, &ctx.task_iri);
            hook_ctx.error = Some(result.summary.clone());
            self.hook_manager
                .execute(HookPoint::AgentError, &mut hook_ctx)
                .await;
        }

        Ok(result)
    }



    async fn exec(
        &self,
        agent: &AgentInstance,
        ctx: TaskContext,
        sess: &mut L1Session,
    ) -> Result<TaskResult, CoreError> {
        let model = self
            .gateway
            .get_model(&agent.role.to_string().to_lowercase());
        let supports_reasoning = self.gateway.supports_native_reasoning(&model);

        let context_data = self.gather_context_data_async(agent.role, &ctx).await;
        let agent_md = self.build_agent_md(agent.role, &ctx.objective, &context_data, &model);

        // 使用 SystemPromptBuilder 构建系统提示词
        let mut prompt_builder = SystemPromptBuilder::new();

        // Region 1: 角色定义区
        prompt_builder.set_region(SystemPromptRegion::RoleDefinition, agent_md.clone());

        // Region 2: 行为准则区（宪法层 + 方法论层）
        {
            let mut policy_text = build_constitution_prompt(agent.role);

            policy_text.push_str("\n\n### 🔴 任务专注原则（必须遵守）\n");
            policy_text.push_str("- 你的唯一任务是当前指定的「当前任务」，工作区中的其他任何目录/文件都与你的任务无关\n");
            policy_text.push_str("- 对于不相关的文件或目录（如其他项目、测试产出、无关代码库），必须直接忽略，禁止探索或处理\n");
            policy_text.push_str("- 使用 glob_search、file_list 或类似工具时，如果结果中包含无关内容，必须自动过滤，禁止被其分散注意力\n");
            policy_text.push_str("- 如果遇到任何不属于当前任务的文件/目录，必须跳过它们，继续执行当前任务，不得因无关内容改变任务方向\n");
            policy_text.push_str("- 检查Agent(CA) 特别注意：你的审计报告只能包含与当前任务相关的内容，发现无关文件时必须忽略，不得写入报告\n");
            policy_text.push_str("- 决策Agent(AA) 特别注意：禁止主动探索文件，你的决策必须仅基于 CA 审计结果，忽略审计结果中的任何无关内容\n");

            // 注入方法论纪律（PA/CA/AA 专属）
            if let Some(methodology_addendum) = MethodologyPromptInjector::build_for_role(agent.role) {
                policy_text.push_str(&methodology_addendum);
            }
            // 注入活跃方法论的劝导指令
            if let Some(ref gate) = self.methodology_gate {
                let directives = gate.inner().read().persuasive_directives();
                if !directives.is_empty() {
                    policy_text.push_str("\n\n### 方法论执行要求\n");
                    for d in &directives {
                        policy_text.push_str(&format!("- {}\n", d));
                    }
                }
            }
            // AA 专属：注入方法论进化简报
            if agent.role == AgentRole::Act {
                if let Some(ref gate) = self.methodology_gate {
                    if let Some(ref evo) = gate.evolution_handle() {
                        let briefing = evo.inner().read().aa_evolution_briefing();
                        if !briefing.is_empty() {
                            policy_text.push_str("\n\n");
                            policy_text.push_str(&briefing);
                        }
                    }
                }
            }
            prompt_builder.set_region(SystemPromptRegion::BehavioralPolicy, policy_text);
        }

        // Region 3: 强调约束区（从 L0 加载）
        let emphasis_items = self.load_emphasis_from_l0(&ctx.task_iri).await;
        if !emphasis_items.is_empty() {
            let emphasis_content = emphasis_items
                .iter()
                .map(|e| format!("- {}", e))
                .collect::<Vec<_>>()
                .join("\n");
            prompt_builder.set_region(SystemPromptRegion::EmphasizedConstraints, emphasis_content);
        }

        // Region 3: 输出格式区
        let format_constraint = if supports_reasoning {
            LLM_RESPONSE_FORMAT_NO_THOUGHT.to_string()
        } else {
            LLM_RESPONSE_FORMAT_WITH_THOUGHT.to_string()
        };
        prompt_builder.set_region(SystemPromptRegion::OutputFormat, format_constraint);

        // Region 4: 输出管理区
        prompt_builder.set_region(
            SystemPromptRegion::OutputManagement,
            crate::core::system_prompt::OUTPUT_MANAGEMENT.to_string(),
        );

        // Region 5: 工具区（内置工具 + 动态工具）
        let tool_menu = self.build_readable_tool_menu(&agent.role);
        if !tool_menu.is_empty() {
            prompt_builder.set_region(SystemPromptRegion::Tools, tool_menu);
        }

        // Region 5: 提取提示区（从配置加载）
        if let Some(ref config) = self.emphasis_config {
            if config.enabled {
                prompt_builder.set_region(
                    SystemPromptRegion::ExtractionPrompt,
                    config.extraction_prompt.clone(),
                );
            }
        }

        // 构建系统提示词（相对固定，放在 system role）
        let system_content = prompt_builder.build();

        // 构建上下文消息（动态变化，放在最后的 user role）
        let summary_iris = sess.get_summary_chain_with_iris(20, 100);
        let summary_text = summary_iris.join("\n");

        let context_msg = if summary_text.is_empty() {
            format!(
                "## 当前任务\n{}\n\n## 可用工具\n请根据需要使用工具完成任务。",
                ctx.objective
            )
        } else {
            format!(
                "## 当前任务\n{}\n\n## 历史摘要\n{}\n\n如果需要查看某轮次的完整报告，可使用 read_agent_output 工具查询对应的 IRI。\n\n## 可用工具\n请根据需要使用工具完成任务。",
                ctx.objective, summary_text
            )
        };

        let mut messages: Vec<ChatMessage> = vec![
            ChatMessage {
                role: "system".to_string(),
                content: system_content,
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
            },
        ];

        // Resume 模式：从 checkpoint 恢复历史消息，放在 system 之后、新 user 消息之前
        // 这样 LLM 先看到历史上下文，再看到继续指令
        if let Some(ref resumed) = ctx.resumed_messages {
            // 跳过原 system 消息（已用新的替换），追加其余历史
            for msg in resumed.iter().skip(1) {
                messages.push(msg.clone());
            }
            info!("[resume] 从 checkpoint 恢复 {} 条历史消息", resumed.len().saturating_sub(1));
        }

        // 新的 user 消息放在历史之后，作为继续指令
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: if ctx.resumed_messages.is_some() {
                format!(
                    "[继续执行] 请从上次中断处继续完成任务。\n\n当前任务: {}",
                    ctx.objective
                )
            } else {
                context_msg
            },
            name: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
        });

        let tools = self
            .tool_executor
            .read()
            .expect("tool_executor RwLock poisoned")
            .tool_definitions_for_role(&agent.role.to_string());

        info!(
            "AgentRunner 开始: role={}, model={}, tools={}, supports_reasoning={}",
            agent.role,
            model,
            tools.len(),
            supports_reasoning
        );

        let mut tc = ctx.resumed_tool_count;
        let mut errs = Vec::new();
        let mut turn = ctx.resumed_turn_count;
        let mut consecutive_failures = 0u32;
        let mut recovery_mode_active = false;
        let mut guard_pending_pre_injections: Vec<String> = Vec::new();
        // 跟踪每个工具的错误次数，同工具反复失败时提前终止
        let mut tool_error_counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        let mut action_tracker = crate::core::tracked_action::ActionTracker::new(
            &ctx.task_iri,
            &agent.role.to_string(),
        );
        let checkpoint_manager = crate::core::checkpoint::CheckpointManager::with_persistence(self.l0_store.clone());

        let effective_max_turns = match agent.role {
            AgentRole::Plan => ctx.max_iterations.min(200),
            AgentRole::Do => ctx.max_iterations,
            AgentRole::Check => ctx.max_iterations.min(200),
            AgentRole::Act => ctx.max_iterations.min(150),
        };

        // 初始 checkpoint：记录任务开始状态
        if let Err(e) = checkpoint_manager.create(
            &ctx.task_iri,
            &format!("start_{}", agent.role),
            "[]",
            &serde_json::to_string(&messages).unwrap_or_default(),
            &serde_json::json!({
                "turn": ctx.resumed_turn_count,
                "tc": ctx.resumed_tool_count,
                "prompt_tokens": self.total_prompt_tokens.load(Ordering::Relaxed),
                "completion_tokens": self.total_completion_tokens.load(Ordering::Relaxed),
            }).to_string(),
            &[agent.role.to_string()],
        ) {
            warn!("[checkpoint] 初始保存失败: {}", e);
        }

        'react_loop: loop {
            if turn >= effective_max_turns {
                warn!("[turn {}] 达到角色 {} 最大轮次限制 {}, 强制结束", turn, agent.role, effective_max_turns);
                errs.push("max turns reached".to_string());
                if let Some(ref event_bus) = self.event_bus {
                    let _ = event_bus.emit(&ctx.task_iri, "AGENT_BLOCKED", &agent.agent_id, &serde_json::json!({"iterations": turn}).to_string()).await;
                }
                // 保存 checkpoint（失败退出前记录状态）
                if let Err(e) = checkpoint_manager.create(
                    &ctx.task_iri,
                    &format!("max_turns_{}", agent.role),
                    "[]",
                    &serde_json::to_string(&messages).unwrap_or_default(),
                    &serde_json::json!({
                        "turn": turn,
                        "tc": tc,
                        "prompt_tokens": self.total_prompt_tokens.load(Ordering::Relaxed),
                        "completion_tokens": self.total_completion_tokens.load(Ordering::Relaxed),
                    }).to_string(),
                    &[agent.role.to_string()],
                ) {
                    warn!("[checkpoint] max_turns 保存失败: {}", e);
                }
                break;
            }
            turn += 1;

            // 每 5 轮保存一次周期 checkpoint
            if turn % 5 == 0 {
                if let Err(e) = checkpoint_manager.create(
                    &ctx.task_iri,
                    &format!("turn_{}_{}", agent.role, turn),
                    "[]",
                    &serde_json::to_string(&messages).unwrap_or_default(),
                    &serde_json::json!({
                        "turn": turn,
                        "tc": tc,
                        "prompt_tokens": self.total_prompt_tokens.load(Ordering::Relaxed),
                        "completion_tokens": self.total_completion_tokens.load(Ordering::Relaxed),
                    }).to_string(),
                    &[agent.role.to_string()],
                ) {
                    warn!("[checkpoint] 周期保存失败 (turn={}): {}", turn, e);
                }
            }

            // 失败模式检测与恢复模式
            if consecutive_failures >= 3 && !recovery_mode_active {
                recovery_mode_active = true;
                let recovery_msg = format!(
                    "[系统诊断] 检测到连续 {} 次操作失败。请暂停执行，分析失败原因，提出不同的解决思路。\
                     \n\n失败记录：{}\n\n请重新评估当前方法，考虑替代方案后再继续。",
                    consecutive_failures,
                    errs.last().map(|e| e.as_str()).unwrap_or("多次失败")
                );
                messages.push(ChatMessage {
                    role: "user".to_string(),
                    content: recovery_msg,
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    reasoning_content: None,
                });
                info!("[consecutive_failures] 触发恢复模式: 连续 {} 次失败", consecutive_failures);
                consecutive_failures = 0;
                continue;
            }

            // ===== Thought 阶段 =====
            info!("[ReAct Turn {}] ===== Thought =====", turn);

            // CycleStart: 注入补充输入（SA 写入 → AgentRunner 消费）
            {
                let pending = self.supplement_store.take_pending(&ctx.task_iri);
                if !pending.is_empty() {
                    info!(
                        task_iri = %ctx.task_iri,
                        count = pending.len(),
                        "注入 {} 条补充输入到 AgentRunner 上下文",
                        pending.len()
                    );
                    for entry in &pending {
                        messages.push(ChatMessage {
                            role: "user".to_string(),
                            content: entry.content.clone(),
                            name: None,
                            tool_calls: None,
                            tool_call_id: None,
                            reasoning_content: None,
                        });
                        sess.add_supplement("user", &entry.content, entry.embedding.clone(), Some(entry.relevance_score));
                    }
                }
            }

            // CycleStart hook
            {
                let mut hook_ctx = HookContext::new(
                    HookPoint::CycleStart,
                    &agent.agent_id,
                    &agent.role.to_string(),
                )
                .with_task(&ctx.task_iri, &ctx.task_iri)
                .with_data("turn", Value::Number(turn.into()));
                self.hook_manager
                    .execute(HookPoint::CycleStart, &mut hook_ctx)
                    .await;
            }

            {
                let mut hook_ctx = HookContext::new(
                    HookPoint::LlmRequest,
                    &agent.agent_id,
                    &agent.role.to_string(),
                )
                .with_task(&ctx.task_iri, &ctx.task_iri);
                let hook_result = self
                    .hook_manager
                    .execute(HookPoint::LlmRequest, &mut hook_ctx)
                    .await;
                if hook_result == HookResult::Abort {
                    errs.push("LLM request aborted by hook".to_string());
                    break;
                }
            }

            // 使用 ContextWindowManager 做基于消息数和 token 的双维度压缩决策
            let context_window_compressed = if let Some(ref cwm_lock) = self.context_window_manager {
                let cwm = cwm_lock.lock().expect("cwm_lock Mutex poisoned");
                if cwm.should_compress(messages.len(), &messages) {
                    let (compressed, summary_text) = cwm.compress_messages(&messages);
                    if !summary_text.is_empty() {
                        sess.add_summary("system", &summary_text, None);
                    }
                    info!(
                        "[turn {}] ContextWindowManager 压缩: {} -> {} 条消息",
                        turn,
                        messages.len(),
                        compressed.len()
                    );
                    Some(compressed)
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(compressed) = context_window_compressed {
                messages = compressed;
            } else if messages.len() > 30 {
                // 回退：纯硬截断（仅在 CWM 不可用时或配置不当的情况下触发）
                let system_msg = messages.first().cloned();
                let kept_recent = messages.len().saturating_sub(15);

                let mut recent: Vec<_> = messages.drain(kept_recent..).collect();

                while !recent.is_empty() {
                    let first = &recent[0];
                    if first.role == "tool" {
                        recent.remove(0);
                        continue;
                    }
                    if first.role == "assistant" {
                        if let Some(ref tool_calls) = first.tool_calls {
                            let expected_tool_results = tool_calls.len();
                            let actual_tool_results = recent
                                .iter()
                                .skip(1)
                                .take_while(|m| m.role == "tool")
                                .count();
                            if actual_tool_results < expected_tool_results {
                                recent.remove(0);
                                continue;
                            }
                        }
                    }
                    break;
                }

                messages.clear();
                if let Some(sys) = system_msg {
                    messages.push(sys);
                }

                let summary_iris = sess.get_summary_chain_with_iris(10, 100);
                let summary_text = if summary_iris.is_empty() {
                    format!(
                        "[历史摘要] 之前已执行 {} 轮操作，包含 {} 次工具调用。以下是最近的对话：",
                        turn - 1, tc
                    )
                } else {
                    format!(
                        "[历史摘要] 已执行 {} 轮。关键记录：\n{}\n\n如需详细信息，使用 kg_search / knowledge_query 查询 IRI。",
                        turn - 1,
                        summary_iris.join("\n")
                    )
                };

                let summary_note = ChatMessage {
                    role: "user".to_string(),
                    content: summary_text,
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    reasoning_content: None,
                };
                messages.push(summary_note);
                messages.extend(recent);

                info!(
                    "[turn {}] 消息历史截断: 保留 {} 条 (原始 {} 条)",
                    turn,
                    messages.len(),
                    kept_recent + 17
                );
            }

            if !guard_pending_pre_injections.is_empty() {
                let prompt = format!(
                    "\n\n[ToolGuard 约束指令]\n{}\n注意：以上约束仅适用于你接下来发起的同名工具调用。请严格遵守。",
                    guard_pending_pre_injections.join("\n")
                );
                if let Some(sys_msg) = messages.first_mut() {
                    if sys_msg.role == "system" {
                        // 替换而非追加：移除旧的 ToolGuard 块，防止每轮累积膨胀
                        if let Some(pos) = sys_msg.content.find("\n\n[ToolGuard 约束指令]") {
                            sys_msg.content.truncate(pos);
                        }
                        sys_msg.content.push_str(&prompt);
                    }
                }
                guard_pending_pre_injections.clear();
            }

            debug!(
                "[turn {}] 调用 LLM (history: {} msgs, tools: {})",
                turn,
                messages.len(),
                tools.len()
            );

            let response = self
                .gateway
                .chat_with_params(
                    &model,
                    messages.clone(),
                    None,
                    None,
                    {
                        // 每次调用前刷新 tools 列表，确保新注册的微工具（如 read_full_result_*）被包含
                        let current_tools = self
                            .tool_executor
                            .read()
                            .expect("tool_executor RwLock poisoned")
                            .tool_definitions_for_role(&agent.role.to_string());
                        if current_tools.is_empty() { None } else { Some(current_tools) }
                    },
                    None,
                )
                .await
                .map_err(|e| CoreError::Internal {
                    message: e.to_string(),
                })?;

            // Accumulate token usage
            if let Some(ref usage) = response.usage {
                self.total_prompt_tokens.fetch_add(usage.prompt_tokens as u64, Ordering::Relaxed);
                self.total_completion_tokens.fetch_add(usage.completion_tokens as u64, Ordering::Relaxed);
            }

            {
                let mut hook_ctx = HookContext::new(
                    HookPoint::LlmResponse,
                    &agent.agent_id,
                    &agent.role.to_string(),
                )
                .with_task(&ctx.task_iri, &ctx.task_iri);
                self.hook_manager
                    .execute(HookPoint::LlmResponse, &mut hook_ctx)
                    .await;
            }

            let choice = response
                .choices
                .first()
                .ok_or_else(|| CoreError::Internal {
                    message: "No choices in response".to_string(),
                })?;
            let raw_content = choice.message.content.clone().unwrap_or_default();
            let reasoning_content = choice.message.reasoning_content.clone();
            let finish = choice.finish_reason.as_deref().unwrap_or("");

            debug!(
                "[turn {}] LLM 回复: finish={}, content_len={}, has_reasoning={}",
                turn,
                finish,
                raw_content.len(),
                reasoning_content.is_some()
            );

            let parsed = self.parse_llm_response(
                &raw_content,
                reasoning_content.as_deref(),
                supports_reasoning,
            );

            // 只有当 finish 不是 tool_calls 时才打印 WARN
            // 因为工具调用时 content 非 JSON 是正常行为
            if !parsed.is_valid_json && finish != "tool_calls" {
                warn!("[turn {}] LLM 回复不是有效 JSON，使用 fallback 处理", turn);
                consecutive_failures += 1;
                debug!("[consecutive_failures] JSON parse failed: {}/3", consecutive_failures);
            }

            let mut action = parsed
                .action
                .clone()
                .unwrap_or_else(|| "continue".to_string());

            if finish == "tool_calls" && choice.message.tool_calls.is_some() {
                action = "tool_call".to_string();
                debug!(
                    "[turn {}] finish=tool_calls 且存在 tool_calls，强制 action=tool_call",
                    turn
                );
            }

            if (finish == "stop" || finish == "end_turn") && action != "tool_call" {
                if action != "finish" {
                    debug!(
                        "[turn {}] finish={} 且无工具调用，将 action 从 {} 修正为 finish",
                        turn, finish, action
                    );
                }
                action = "finish".to_string();
            }

            info!(
                "[ReAct Turn {}] Thought: action={}, summary={}",
                turn,
                action,
                parsed.summary.as_deref().unwrap_or("")
            );

            // Emit thought event to event bus for TUI display
            if let Some(ref event_bus) = self.event_bus {
                let thought_content = parsed.thought.clone().unwrap_or_default();
                let thought_event = ExecutionEvent {
                    event_id: format!("evt_{}", uuid::Uuid::new_v4().hyphenated()),
                    task_iri: ctx.task_iri.clone(),
                    timestamp: chrono::Utc::now().timestamp_millis(),
                    event: ExecutionEventKind::Thought(crate::core::execution_event::Thought {
                        agent_id: agent.agent_id.clone(),
                        thought: if thought_content.is_empty() { parsed.content.clone() } else { thought_content },
                        action: action.clone(),
                        emphasis: parsed.emphasis.clone(),
                    }),
                };
                let _ = event_bus.emit(
                    &ctx.task_iri,
                    "THOUGHT",
                    &agent.agent_id,
                    &serde_json::to_string(&thought_event).unwrap_or_default(),
                ).await;
            }

            // 保存强调内容到 L0 永久记忆
            if !parsed.emphasis.is_empty() {
                let dedup_threshold = self
                    .emphasis_config
                    .as_ref()
                    .map(|c| c.dedup_threshold)
                    .unwrap_or(0.85);
                self.save_emphasis_to_l0(
                    &parsed.emphasis,
                    &ctx.task_iri,
                    &agent.agent_id,
                    dedup_threshold,
                )
                .await;
            }

            // 归档到 L0：保存完整回复 + 思考内容
            let l0_iri = sess
                .archive_full_to_l0(
                    &self.l0_store,
                    &agent.role.to_string(),
                    &parsed.thought.clone().unwrap_or_default(),
                    &parsed.content,
                )
                .ok();
            debug!(
                "[L0] 归档: {:?}, has_reasoning={}, is_valid_json={}",
                l0_iri, parsed.has_native_reasoning, parsed.is_valid_json
            );

            // MemoryWrite hook for L0 archive
            {
                let mut hook_ctx = HookContext::new(
                    HookPoint::MemoryWrite,
                    &agent.agent_id,
                    &agent.role.to_string(),
                )
                .with_task(&ctx.task_iri, &ctx.task_iri)
                .with_data("storage", Value::String("L0".to_string()));
                if let Some(ref iri) = l0_iri {
                    hook_ctx
                        .data
                        .insert("iri".to_string(), Value::String(iri.clone()));
                }
                self.hook_manager
                    .execute(HookPoint::MemoryWrite, &mut hook_ctx)
                    .await;
            }

            let task_id = ctx.task_iri
                .strip_prefix("iri://task/")
                .unwrap_or_else(|| ctx.task_iri.strip_prefix("iri://").unwrap_or(&ctx.task_iri));
            let node_iri = format!(
                "iri://task/{}/turn_{}",
                task_id,
                turn
            );
            let mut node_json = json!({
                "@id": &node_iri,
                "@type": "AgentTurn",
                "role": agent.role.to_string(),
                "content": parsed.content,
                "content_len": parsed.content.len(),
                "is_valid_json": parsed.is_valid_json,
                "has_native_reasoning": parsed.has_native_reasoning
            });
            if let Some(ref thought) = parsed.thought {
                node_json["has_thought"] = Value::Bool(true);
                node_json["thought_len"] = Value::Number(thought.len().into());
            }
            if let Some(ref act) = parsed.action {
                node_json["action"] = Value::String(act.clone());
            }
            if let Some(ref s) = parsed.summary {
                node_json["summary"] = Value::String(s.clone());
            }
            JsonLdContext::inject(&mut node_json);
            let cfg = crate::CoreConfig::default();
            match self
                .blackboard
                .write_node(&node_iri, &node_json.to_string(), &cfg)
            {
                Ok(_) => {
                    debug!("[L2] 写入节点: {}", node_iri);

                    // BlackboardWrite hook
                    let mut hook_ctx = HookContext::new(
                        HookPoint::BlackboardWrite,
                        &agent.agent_id,
                        &agent.role.to_string(),
                    )
                    .with_task(&ctx.task_iri, &ctx.task_iri)
                    .with_data("node_iri", Value::String(node_iri.clone()));
                    self.hook_manager
                        .execute(HookPoint::BlackboardWrite, &mut hook_ctx)
                        .await;
                }
                Err(e) => {
                    warn!("[L2] 写入节点失败 {}: {:?}", node_iri, e);
                }
            }

            // 使用解析后的 summary 或生成 fallback
            let summary_text = parsed
                .summary
                .clone()
                .unwrap_or_else(|| Self::generate_auto_summary(&parsed.content));
            let l1_turn = sess.add_summary(&agent.role.to_string(), &summary_text, l0_iri.clone());
            // 计算 turn embedding 和 relevance_score
            if let (Some(ref embedder), Some(ref tracker_lock)) = (&self.embedder, &self.relevance_tracker) {
                if let Ok(emb) = embedder.embed(&summary_text).await {
                    let mut tracker = tracker_lock.lock().unwrap();
                    let score = tracker.on_new_input(&emb);
                    l1_turn.embedding = Some(emb);
                    l1_turn.relevance_score = Some(score);
                }
            }

            // ===== Action 阶段 =====
            info!("[ReAct Turn {}] ===== Action =====", turn);

            match action.as_str() {
                "finish" => {
                    info!("[ReAct] Agent 决定完成任务");

                    // CycleEnd hook
                    {
                        let mut hook_ctx = HookContext::new(
                            HookPoint::CycleEnd,
                            &agent.agent_id,
                            &agent.role.to_string(),
                        )
                        .with_task(&ctx.task_iri, &ctx.task_iri)
                        .with_data("turn", Value::Number(turn.into()))
                        .with_data("had_tool_calls", Value::Bool(false));
                        self.hook_manager
                            .execute(HookPoint::CycleEnd, &mut hook_ctx)
                            .await;
                    }

                    info!("AgentRunner 完成: {} turns, {} tools", turn, tc);
                    debug!("[L0] L0 entries: {}", self.l0_store.count());

                    let final_summary = parsed.summary.clone()
                        .unwrap_or_else(|| Self::generate_auto_summary(&parsed.content));

                    let output_value = Value::String(parsed.content.clone());
                    let jsonld_output =
                        self.apply_output_mapping(&output_value, &agent.role, &ctx.task_iri);

                    if let Some(ref jsonld) = jsonld_output {
                        if let Ok(node) = JsonLdNode::from_json(jsonld) {
                            let emphasis_items = self.extract_emphasis(&node);
                            if !emphasis_items.is_empty() {
                                let dedup_threshold = self
                                    .emphasis_config
                                    .as_ref()
                                    .map(|c| c.dedup_threshold)
                                    .unwrap_or(0.85);
                                self.save_emphasis_to_l0(
                                    &emphasis_items,
                                    &ctx.task_iri,
                                    &agent.agent_id,
                                    dedup_threshold,
                                )
                                .await;
                            }

                            if let Ok(node_iri) =
                                self.store_jsonld_to_l2(&node, &ctx.task_iri).await
                            {
                                debug!("[L2] JSON-LD 输出已存储: {}", node_iri);
                            }
                        }
                    }

                    let nodes_str = jsonld_output
                        .as_ref()
                        .map(|j| j.to_string())
                        .unwrap_or_else(|| "[]".to_string());
                    if let Err(e) = checkpoint_manager.create(
                        &ctx.task_iri,
                        &format!("finish_{}", agent.role),
                        &nodes_str,
                        &serde_json::to_string(&messages).unwrap_or_default(),
                        &serde_json::json!({
                            "turn": turn,
                            "tc": tc,
                            "prompt_tokens": self.total_prompt_tokens.load(Ordering::Relaxed),
                            "completion_tokens": self.total_completion_tokens.load(Ordering::Relaxed),
                        }).to_string(),
                        &[agent.role.to_string()],
                    ) {
                        warn!("[checkpoint] finish 保存失败: {}", e);
                    }

                    return Ok(TaskResult {
                        task_iri: ctx.task_iri,
                        status: "success".to_string(),
                        summary: final_summary,
                        output: Some(output_value),
                        jsonld_output,
                        artifacts: vec![],
                        errors: errs,
                        turn_count: turn,
                        tool_call_count: tc,
                        five_w2h_updates: None,
                        tracked_actions: action_tracker.actions,
                        archive_iri: Some(node_iri.clone()),
                    });
                }
                "tool_call" => {
                    if let Some(calls) = &choice.message.tool_calls {
                        let tool_names: Vec<&str> =
                            calls.iter().map(|c| c.function.name.as_str()).collect();
                        debug!("[tool_calls] {} → {:?}", calls.len(), tool_names);

                        // 🔴 PA角色禁止调用写操作工具，但允许只读工具
                        if agent.role == AgentRole::Plan {
                            let write_tools: Vec<&str> = calls
                                .iter()
                                .map(|c| c.function.name.as_str())
                                .filter(|name| !ToolExecutor::is_pa_readonly_tool(name))
                                .collect();

                            let force_finish = if let Some(ref tc) = self.tool_controller {
                                let tool_calls: Vec<(String, Value)> = calls.iter()
                                    .map(|c| (c.function.name.clone(), serde_json::from_str(&c.function.arguments).unwrap_or_default()))
                                    .collect();
                                tc.should_force_finish(&tool_calls, &agent.role)
                            } else {
                                !write_tools.is_empty()
                            };

                            if force_finish {
                                warn!(
                                    "[PA] 检测到写操作工具调用: {:?}，强制转换为finish",
                                    write_tools
                                );
                                info!("[ReAct] PA Agent 被强制结束（禁止写操作）");

                                let final_summary = parsed
                                    .summary
                                    .clone()
                                    .unwrap_or_else(|| "PA已制定计划".to_string());

                                let output_value = Value::String(parsed.content.clone());
                                let jsonld_output = self.apply_output_mapping(
                                    &output_value,
                                    &agent.role,
                                    &ctx.task_iri,
                                );

                                return Ok(TaskResult {
                                    task_iri: ctx.task_iri,
                                    status: "success".to_string(),
                                    summary: final_summary,
                                    output: Some(output_value),
                                    jsonld_output,
                                    artifacts: vec![],
                                    errors: errs,
                                    turn_count: turn,
                                    tool_call_count: tc,
                                    five_w2h_updates: None,
                                    tracked_actions: Vec::new(),
                                    archive_iri: Some(node_iri.clone()),
                                });
                            }
                        }

                        let asst_summary = parsed.summary.clone()
                            .unwrap_or_else(|| Self::generate_auto_summary(&parsed.content));
                        messages.push(ChatMessage {
                            role: "assistant".to_string(),
                            content: asst_summary,
                            name: None,
                            tool_calls: Some(
                                calls
                                    .iter()
                                    .map(|c| crate::gateway::unified_gateway::ToolCallPayload {
                                        id: c.id.clone(),
                                        call_type: c.call_type.clone(),
                                        function:
                                            crate::gateway::unified_gateway::ToolCallFunction {
                                                name: c.function.name.clone(),
                                                arguments: c.function.arguments.clone(),
                                            },
                                    })
                                    .collect(),
                            ),
                            tool_call_id: None,
                            reasoning_content: reasoning_content.clone(),
                        });

                        for c in calls {
                            tc += 1;
                            let name = &c.function.name;
                            let args_raw = &c.function.arguments;
                            let args: Value = serde_json::from_str(args_raw).unwrap_or_default();
                            debug!(
                                "  [tool] {} args={}",
                                name,
                                &args_raw.chars().take(100).collect::<String>()
                            );

                            // Emit tool_call event for TUI display
                            if let Some(ref event_bus) = self.event_bus {
                                let tce = ExecutionEvent {
                                    event_id: format!("evt_{}", uuid::Uuid::new_v4().hyphenated()),
                                    task_iri: ctx.task_iri.clone(),
                                    timestamp: chrono::Utc::now().timestamp_millis(),
                                    event: ExecutionEventKind::ToolCall(crate::core::execution_event::ToolCall {
                                        call_id: c.id.clone(),
                                        tool_name: name.clone(),
                                        arguments_json: args_raw.clone(),
                                        agent_id: agent.agent_id.clone(),
                                        sequence: tc,
                                    }),
                                };
                                let _ = event_bus.emit(
                                    &ctx.task_iri,
                                    "TOOL_CALL",
                                    &agent.agent_id,
                                    &serde_json::to_string(&tce).unwrap_or_default(),
                                ).await;
                            }

                            {
                                let mut hook_ctx = HookContext::new(
                                    HookPoint::SkillBefore,
                                    &agent.agent_id,
                                    &agent.role.to_string(),
                                )
                                .with_task(&ctx.task_iri, &ctx.task_iri)
                                .with_data("tool_name", Value::String(name.clone()));
                                self.hook_manager
                                    .execute(HookPoint::SkillBefore, &mut hook_ctx)
                                    .await;
                                // Capture ToolGuard pre-injections for next LLM call
                                if let Some(injections) = hook_ctx.metadata.remove("guard_pre_injections") {
                                    if let Value::Array(arr) = injections {
                                        for v in arr {
                                            if let Some(s) = v.as_str() {
                                                guard_pending_pre_injections.push(s.to_string());
                                            }
                                        }
                                    }
                                }
                            }

                            let handler = {
                                let executor = self.tool_executor.read().unwrap_or_else(|e| {
                                    warn!("ToolExecutor 读锁中毒 (exec handler): {}", e);
                                    e.into_inner()
                                });
                                executor.try_get_handler(name)
                            };
                            let started_at = std::time::Instant::now();
                            let args_clone = args.clone();
                            let result = match handler {
                                Some(f) => f(args).await.unwrap_or_else(|e| json!({"error": e})),
                                None => json!({"error": format!("Tool not found: {}", name)}),
                            };
                            action_tracker.record(name, &args_clone, &result, started_at.elapsed().as_secs_f64());
                            let raw_result_str = serde_json::to_string(&result).unwrap_or_default();

                            let mut result_str = self.route_tool_result(
                                &raw_result_str,
                                name,
                                &c.id,
                            ).await;

                            debug!("  [tool] {} result: {} bytes (raw: {} bytes)", name, result_str.len(), raw_result_str.len());

                            // Emit tool_result event for TUI display
                            if let Some(ref event_bus) = self.event_bus {
                                let tre = ExecutionEvent {
                                    event_id: format!("evt_{}", uuid::Uuid::new_v4().hyphenated()),
                                    task_iri: ctx.task_iri.clone(),
                                    timestamp: chrono::Utc::now().timestamp_millis(),
                                    event: ExecutionEventKind::ToolResult(crate::core::execution_event::ToolResult {
                                        call_id: c.id.clone(),
                                        tool_name: name.clone(),
                                        result: result_str.clone(),
                                        success: result.get("error").is_none(),
                                        result_size_bytes: result_str.len() as u32,
                                        duration_ms: 0,
                                        agent_id: agent.agent_id.clone(),
                                    }),
                                };
                                let _ = event_bus.emit(
                                    &ctx.task_iri,
                                    "TOOL_RESULT",
                                    &agent.agent_id,
                                    &serde_json::to_string(&tre).unwrap_or_default(),
                                ).await;
                            }

                            if let Some(ref compressor_lock) = self.tool_result_compressor {
                                if let Ok(mut compressor) = compressor_lock.lock() {
                                    compressor.add_result(turn, name, &result_str);
                                    compressor.compress_tool_messages(&mut messages);
                                }
                            }
                            self.compress_tool_results_with_microtools(&mut messages);

                            if let Some(err) = result.get("error") {
                                let err_msg = err.as_str().unwrap_or("");
                                let is_tool_not_found = err_msg.starts_with("Tool not found: ");
                                warn!("[tool] {} 失败: {}", name, err);
                                errs.push(format!("{}: {}", name, err));

                                if is_tool_not_found {
                                    // 微工具注册与 handler 不一致导致「找不到工具」。
                                    // 这不属于 LLM 的错误——工具列表是系统告诉它的。不要计入连续失败。
                                    // try_get_handler 已通过 fallback 路径尽力查找，若仍找不到则说明
                                    // 该微工具有效期已过或数据已清理。LLM 应改用原工具（bash/grep 等）
                                    // 加更精确参数来获取所需数据。
                                    // 此外，向 tool 消息注入提示语，引导 LLM 正确操作。
                                    info!("[tool_error] {} 工具不存在（微工具 fallback 也失败），不计入连续失败", name);
                                    // 向 tool 消息注入引导提示，帮助 LLM 改用原工具
                                    result_str = format!(
                                        "{}\n\n提示：工具 {} 当前不可用。请改用原始工具（如 bash、grep_search）加更精确的参数直接获取所需数据，不要重复调用此微工具。",
                                        result_str, name
                                    );
                                } else {
                                    // 工具执行错误不计入 consecutive_failures。
                                    // consecutive_failures 只追踪 LLM 级别的故障（JSON 解析失败等）。
                                    // 工具错误是正常操作反馈——LLM 已收到错误消息并能自主调整策略。
                                    // 同工具反复失败有独立的 tool_error_counts 计数器处理。
                                    let tool_count = tool_error_counts.entry(name.clone()).or_insert(0);
                                    *tool_count += 1;
                                    debug!("[tool_error] {} 失败次数: {}/3", name, *tool_count);
                                    if *tool_count >= 3 {
                                        warn!("[tool_error] {} 连续失败 {} 次，注入恢复引导", name, *tool_count);
                                        // 设哨兵值防止同一工具的重复错误信息挤占上下文
                                        *tool_count = 999;
                                        result_str = format!(
                                            "{}\n\n[系统提示] 工具 {} 连续 3 次执行失败，说明该工具当前不可用。\
                                             \n请改用其他可用工具完成当前目标（如 web_search / bash / grep 等）。\
                                             \n不要再调用 {}。",
                                            result_str, name, name
                                        );
                                    }
                                }
                                if let Some(ref event_bus) = self.event_bus {
                                    let _ = event_bus.emit(&ctx.task_iri, "AGENT_ERROR", &agent.agent_id, &serde_json::json!({"error": err, "tool": name}).to_string()).await;
                                }
                            } else {
                                info!("[tool] {} 成功", name);
                                if recovery_mode_active {
                                    info!("[consecutive_failures] 恢复模式成功退出");
                                }
                                consecutive_failures = 0;
                                recovery_mode_active = false;
                                // 该工具成功执行，清除它的错误计数
                                tool_error_counts.remove(name);
                            }

                            {
                                let mut hook_ctx = HookContext::new(
                                    HookPoint::SkillAfter,
                                    &agent.agent_id,
                                    &agent.role.to_string(),
                                )
                                .with_task(&ctx.task_iri, &ctx.task_iri)
                                .with_data("tool_name", Value::String(name.clone()))
                                .with_data("tool_result", Value::String(raw_result_str.clone()));
                                let hook_result = self.hook_manager
                                    .execute(HookPoint::SkillAfter, &mut hook_ctx)
                                    .await;

                                if hook_result == HookResult::Abort {
                                    let guard_msg = hook_ctx.error.unwrap_or_else(|| "Tool result rejected by guard".to_string());
                                    warn!("[tool] {} ToolGuard 拦截: {}", name, guard_msg);
                                    messages.push(ChatMessage {
                                        role: "tool".to_string(),
                                        content: format!("[ToolGuard 拦截] 工具 {} 的结果被安全系统拒绝。{}", name, guard_msg),
                                        name: None,
                                        tool_calls: None,
                                        tool_call_id: Some(c.id.clone()),
                                        reasoning_content: None,
                                    });
                                } else {
                                    messages.push(ChatMessage {
                                        role: "tool".to_string(),
                                        content: result_str,
                                        name: None,
                                        tool_calls: None,
                                        tool_call_id: Some(c.id.clone()),
                                        reasoning_content: None,
                                    });
                                }
                            }
                        }

                        // ===== Observation 阶段 =====
                        info!("[ReAct Turn {}] ===== Observation =====", turn);

                        // CycleEnd hook (tool calls path)
                        {
                            let mut hook_ctx = HookContext::new(
                                HookPoint::CycleEnd,
                                &agent.agent_id,
                                &agent.role.to_string(),
                            )
                            .with_task(&ctx.task_iri, &ctx.task_iri)
                            .with_data("turn", Value::Number(turn.into()))
                            .with_data("had_tool_calls", Value::Bool(true));
                            self.hook_manager
                                .execute(HookPoint::CycleEnd, &mut hook_ctx)
                                .await;
                        }

                        continue;
                    } else {
                        warn!("[ReAct] action=tool_call 但无 tool_calls，继续思考");
                        let asst_summary = parsed.summary.clone()
                            .unwrap_or_else(|| Self::generate_auto_summary(&parsed.content));
                        messages.push(ChatMessage {
                            role: "assistant".to_string(),
                            content: asst_summary,
                            name: None,
                            tool_calls: None,
                            tool_call_id: None,
                            reasoning_content: reasoning_content.clone(),
                        });

                        // CycleEnd hook
                        {
                            let mut hook_ctx = HookContext::new(
                                HookPoint::CycleEnd,
                                &agent.agent_id,
                                &agent.role.to_string(),
                            )
                            .with_task(&ctx.task_iri, &ctx.task_iri)
                            .with_data("turn", Value::Number(turn.into()))
                            .with_data("had_tool_calls", Value::Bool(false));
                            self.hook_manager
                                .execute(HookPoint::CycleEnd, &mut hook_ctx)
                                .await;
                        }
                    }
                }
                "continue" => {
                    let asst_summary = parsed.summary.clone()
                        .unwrap_or_else(|| Self::generate_auto_summary(&parsed.content));
                    messages.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: asst_summary,
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                        reasoning_content: reasoning_content.clone(),
                    });

                    // CycleEnd hook
                    {
                        let mut hook_ctx = HookContext::new(
                            HookPoint::CycleEnd,
                            &agent.agent_id,
                            &agent.role.to_string(),
                        )
                        .with_task(&ctx.task_iri, &ctx.task_iri)
                        .with_data("turn", Value::Number(turn.into()))
                        .with_data("had_tool_calls", Value::Bool(false));
                        self.hook_manager
                            .execute(HookPoint::CycleEnd, &mut hook_ctx)
                            .await;
                    }
                }
                _ => {
                    warn!("[ReAct] 未知 action: {}, 继续思考", action);
                    let asst_summary = parsed.summary.clone()
                        .unwrap_or_else(|| Self::generate_auto_summary(&parsed.content));
                    messages.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: asst_summary,
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                        reasoning_content: reasoning_content.clone(),
                    });

                    // CycleEnd hook
                    {
                        let mut hook_ctx = HookContext::new(
                            HookPoint::CycleEnd,
                            &agent.agent_id,
                            &agent.role.to_string(),
                        )
                        .with_task(&ctx.task_iri, &ctx.task_iri)
                        .with_data("turn", Value::Number(turn.into()))
                        .with_data("had_tool_calls", Value::Bool(false));
                        self.hook_manager
                            .execute(HookPoint::CycleEnd, &mut hook_ctx)
                            .await;
                    }
                }
            }
        }

        warn!("AgentRunner 未完成: {} turns, errors: {:?}", turn, errs);
        let status = if tc > 0 { "partial_success" } else { "failed" };
        let summary = if tc > 0 {
            format!("任务部分完成。执行了 {} 轮，{} 次工具调用，剩余 {} 轮未完成。错误: {} 个。", turn, tc, effective_max_turns.saturating_sub(turn), errs.len())
        } else {
            String::new()
        };
        Ok(TaskResult {
            task_iri: ctx.task_iri,
            status: status.to_string(),
            summary,
            output: None,
            jsonld_output: None,
            artifacts: vec![],
            errors: errs,
            turn_count: turn,
            tool_call_count: tc,
            five_w2h_updates: None,
                tracked_actions: Vec::new(),
            archive_iri: None,
        })
    }
}
