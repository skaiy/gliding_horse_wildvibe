use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use crate::templates::template_engine::TemplateEngine;

fn builtin_fallback(role: &str) -> &'static str {
    match role {
        "pa" => "你是计划 Agent(PA)。分析任务并制定执行计划。完成后输出 JSON 格式结果。",
        "da" => "你是执行 Agent(DA)。执行任务，创建产物。优先使用 web_search 获取最新信息。完成后输出 JSON 格式结果。",
        "ca" => "你是检查 Agent(CA)。验证执行结果是否满足要求。完成后输出 JSON 格式结果。",
        "aa" => "你是决策 Agent(AA)。基于审计结果做最终决策和总结。完成后输出 JSON 格式结果。",
        _ => "",
    }
}

#[derive(Debug, Clone)]
pub struct PromptConfig {
    pub user_prefix: Option<String>,
    pub role_overrides: HashMap<String, String>,
    pub env_refs: Vec<String>,
}

impl Default for PromptConfig {
    fn default() -> Self {
        Self { user_prefix: None, role_overrides: HashMap::new(), env_refs: Vec::new() }
    }
}

pub struct PromptLoader {
    config: PromptConfig,
    engine: Arc<TemplateEngine>,
}

impl PromptLoader {
    pub fn new(config: PromptConfig, engine: Arc<TemplateEngine>) -> Self {
        Self { config, engine }
    }

    pub fn load(&self, role: &str, template: &str, vars: &HashMap<String, Value>) -> String {
        let fname = format!("{}/{}.md", role, template);

        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_default();
        if !home.is_empty() {
            let path = PathBuf::from(&home).join(".gliding_horse").join("prompts").join(&fname);
            if path.exists() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    return self.post_process(role, &Self::render_string(&content, vars));
                }
            }
        }

        let proj_path = PathBuf::from(".gliding_horse").join("prompts").join(&fname);
        if proj_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&proj_path) {
                return self.post_process(role, &Self::render_string(&content, vars));
            }
        }

        if let Ok(content) = self.engine.render_prompt(role, template, vars, false, None) {
            return self.post_process(role, &content);
        }

        let fallback = builtin_fallback(role);
        self.post_process(role, &Self::render_string(fallback, vars))
    }

    fn post_process(&self, role: &str, content: &str) -> String {
        let mut result = content.to_string();

        for ref_name in &self.config.env_refs {
            if let Ok(val) = std::env::var(ref_name) {
                result = result.replace(&format!("${{{}}}", ref_name), &val);
            }
        }

        if let Some(ref prefix) = self.config.user_prefix {
            result = format!("{}\n\n{}", prefix, result);
        }

        if let Some(ref override_content) = self.config.role_overrides.get(role) {
            result = format!("{}\n\n---\n\n## 用户追加约束\n{}", result, override_content);
        }

        result
    }

    pub fn render_string(template: &str, variables: &HashMap<String, Value>) -> String {
        let mut result = template.to_string();
        for (key, value) in variables {
            let placeholder = format!("{{{}}}", key);
            let replacement = match value {
                Value::String(s) => s.clone(),
                Value::Object(_) | Value::Array(_) => serde_json::to_string_pretty(value).unwrap_or_default(),
                Value::Null => String::new(),
                _ => value.to_string(),
            };
            result = result.replace(&placeholder, &replacement);
        }
        result
    }
}
