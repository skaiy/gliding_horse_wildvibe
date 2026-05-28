use agent_os::config::GatewaySettings;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct CliConfig {
    pub gateway: GatewaySettings,
    pub model: String,
    pub workspace: String,
    pub max_iterations: u32,
}

impl CliConfig {
    pub fn from_env_and_args(model: String, workspace: String, max_iterations: u32) -> Self {
        let api_key = std::env::var("DEEPSEEK_API_KEY")
            .or_else(|_| std::env::var("AGENT_OS_GATEWAY_API_KEY"))
            .unwrap_or_else(|_| {
                eprintln!("错误: 请设置 DEEPSEEK_API_KEY 或 AGENT_OS_GATEWAY_API_KEY 环境变量");
                std::process::exit(1);
            });

        let base_url = std::env::var("DEEPSEEK_API_URL")
            .or_else(|_| std::env::var("AGENT_OS_GATEWAY_BASE_URL"))
            .unwrap_or_else(|_| "https://api.deepseek.com".to_string());

        let model = if model.is_empty() {
            "deepseek-v4-flash".to_string()
        } else {
            model
        };

        let gateway = GatewaySettings {
            base_url,
            api_key,
            default_model: model.clone(),
            timeout_seconds: 300,
            max_retries: 2,
            model_mapping: HashMap::from([
                ("planning".to_string(), model.clone()),
                ("execution".to_string(), model.clone()),
                ("analysis".to_string(), model.clone()),
                ("default".to_string(), model.clone()),
            ]),
        };

        Self {
            gateway,
            model,
            workspace,
            max_iterations,
        }
    }

    pub fn clone_with_model(&self, model: String) -> Self {
        let gateway = GatewaySettings {
            default_model: model.clone(),
            model_mapping: HashMap::from([
                ("planning".to_string(), model.clone()),
                ("execution".to_string(), model.clone()),
                ("analysis".to_string(), model.clone()),
                ("default".to_string(), model.clone()),
            ]),
            ..self.gateway.clone()
        };

        Self {
            gateway,
            model,
            workspace: self.workspace.clone(),
            max_iterations: self.max_iterations,
        }
    }
}