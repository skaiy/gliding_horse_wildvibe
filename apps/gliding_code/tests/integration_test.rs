use code_cli::config::CliConfig;
use code_cli::engine::CodeCliEngine;

use agent_os::config::GatewaySettings;

#[test]
fn test_config_from_env() {
    std::env::set_var("DEEPSEEK_API_KEY", "test-key");
    std::env::set_var("DEEPSEEK_API_URL", "https://api.deepseek.com");

    let config = CliConfig::from_env_and_args(
        "deepseek-v4-flash".to_string(),
        "/tmp".to_string(),
        20,
    );

    assert_eq!(config.model, "deepseek-v4-flash");
    assert_eq!(config.workspace, "/tmp");
    assert_eq!(config.max_iterations, 20);
    assert_eq!(config.gateway.default_model, "deepseek-v4-flash");
    assert_eq!(config.gateway.base_url, "https://api.deepseek.com");
}

#[test]
fn test_config_model_switch() {
    std::env::set_var("DEEPSEEK_API_KEY", "test-key");

    let config = CliConfig::from_env_and_args(
        "deepseek-v4-flash".to_string(),
        ".".to_string(),
        20,
    );

    let new_config = config.clone_with_model("deepseek-v4-pro".to_string());
    assert_eq!(new_config.model, "deepseek-v4-pro");
    assert_eq!(new_config.gateway.default_model, "deepseek-v4-pro");
}

#[test]
fn test_engine_build() {
    std::env::set_var("DEEPSEEK_API_KEY", "test-key");

    let config = CliConfig::from_env_and_args(
        "deepseek-v4-flash".to_string(),
        ".".to_string(),
        20,
    );

    let engine = CodeCliEngine::new(config);
    assert!(engine.is_ok(), "引擎构建应成功");
}

fn get_real_config() -> CliConfig {
    let api_key = std::env::var("DEEPSEEK_API_KEY")
        .expect("DEEPSEEK_API_KEY must be set");
    let base_url = std::env::var("DEEPSEEK_API_URL")
        .unwrap_or_else(|_| "https://api.deepseek.com".to_string());

    let gateway = GatewaySettings {
        base_url,
        api_key,
        default_model: "deepseek-v4-flash".to_string(),
        timeout_seconds: 120,
        max_retries: 2,
        model_mapping: Default::default(),
    };

    CliConfig {
        gateway,
        model: "deepseek-v4-flash".to_string(),
        workspace: "/tmp/code_cli_test".to_string(),
        max_iterations: 10,
    }
}

#[tokio::test]
#[ignore]
async fn test_e2e_sa_task() {
    let config = get_real_config();
    let mut engine = CodeCliEngine::new(config).expect("引擎构建失败");

    let result = engine.process_task("Say hello in one word").await;

    match result {
        Ok((task_iri, result)) => {
            println!("Task IRI: {}", task_iri);
            println!("Status: {}", result.status);
            println!("Summary: {}", result.summary);
            assert!(!result.summary.is_empty() || result.status == "success");
        }
        Err(e) => {
            println!("Error: {}", e);
        }
    }
}

#[tokio::test]
#[ignore]
async fn test_e2e_model_switch() {
    let mut config = get_real_config();

    let mut engine = CodeCliEngine::new(config.clone()).expect("引擎构建失败");

    let result1 = engine.process_task("Say 'first'").await;
    println!("Flash result: {:?}", result1.map(|(_, r)| r.status));

    config = config.clone_with_model("deepseek-v4-pro".to_string());
    let mut engine2 = CodeCliEngine::new(config).expect("pro 引擎构建失败");

    let result2 = engine2.process_task("Say 'second'").await;
    println!("Pro result: {:?}", result2.map(|(_, r)| r.status));
}
