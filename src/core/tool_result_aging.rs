use std::sync::RwLock;
use tracing::debug;

use crate::config::settings::ToolResultAgingSettings;
use crate::gateway::unified_gateway::ChatMessage;

/// 对 messages[] 中的旧 tool 结果按轮次深度自动降级。
///
/// 策略:
/// - 保留最近的 N 条完整 tool 结果 (keep_full)
/// - 更旧的 tool 结果如果有对应微工具 → 替换为引用式压缩
/// - 最旧的 tool 结果 → 替换为简短摘要
///
/// 定位依据: messages[] 是按时间序排列的，从前向后扫描即为从旧到新。
#[derive(Clone)]
pub struct ToolResultAging {
    /// 保留完整结果的数量（最近这些保持完整）
    keep_full: usize,
    /// 对旧结果尝试微工具引用的数量（在此范围内的尝试引用式压缩）
    try_microtool: usize,
    /// 压缩阈值: 超过此字节数的 tool 消息才被处理
    compress_threshold: usize,
}

impl ToolResultAging {
    pub fn new(settings: &ToolResultAgingSettings) -> Self {
        Self {
            keep_full: settings.keep_full,
            try_microtool: settings.try_microtool,
            compress_threshold: settings.compress_threshold,
        }
    }

    /// 对 messages 中的 tool 消息按陈旧度做自动降级压缩。
    ///
    /// 返回 (aged_count, freed_bytes)
    pub fn age_tool_results(
        &self,
        messages: &mut Vec<ChatMessage>,
        tool_executor: &RwLock<crate::tools::tool_executor::ToolExecutor>,
    ) -> (usize, usize) {
        // 收集所有 tool 消息的索引（跳过开头的 system/perception 等非 tool 消息）
        let tool_indices: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == "tool")
            .map(|(i, _)| i)
            .collect();

        let total = tool_indices.len();
        if total <= self.keep_full {
            return (0, 0);
        }

        let mut aged = 0usize;
        let mut freed = 0usize;

        // 按时间序从旧到新处理：最新的 N 条保持完整，之前的按陈旧度降级
        // batch_idx=0=最旧, batch_idx=N-1=最新
        let microtool_end = self.keep_full + self.try_microtool;

        for (batch_idx, &msg_idx) in tool_indices.iter().enumerate() {
            // 从最新向后数的位置: rev_position=0=最新
            let rev_position = total - 1 - batch_idx;
            if rev_position < self.keep_full {
                continue; // 最新的 N 条保持完整
            }

            let msg = &messages[msg_idx];
            if msg.content.len() < self.compress_threshold {
                continue; // 小结果不处理
            }

            let call_id = match msg.tool_call_id.as_deref() {
                Some(id) if !id.is_empty() => id.to_string(),
                _ => continue,
            };

            let original_len = msg.content.len();

            if rev_position < microtool_end {
                // 次旧批次：尝试微工具引用式压缩
                let micro_tool_name = format!("read_full_result_{}", call_id);
                let has_micro_tool = tool_executor
                    .read()
                    .ok()
                    .and_then(|exe| exe.try_get_handler(&micro_tool_name))
                    .is_some();

                if has_micro_tool {
                    let iri = format!("iri://tool-result/{}", call_id);
                    messages[msg_idx].content = format!(
                        "[已压缩 {} 字节] 完整结果请调用 `{}` 工具\nIRI: {}",
                        original_len, micro_tool_name, iri,
                    );
                } else {
                    // 无微工具，用简短摘要
                    let preview: String = msg.content.chars().take(150).collect();
                    messages[msg_idx].content = format!(
                        "[旧结果 {} 字节] {}...",
                        original_len, preview
                    );
                }
            } else {
                // 最旧批次：直接替换为简短摘要
                let preview: String = msg.content.chars().take(100).collect();
                messages[msg_idx].content = format!(
                    "[历史结果 {} 字节] {}...",
                    original_len, preview
                );
            }

            freed += original_len.saturating_sub(messages[msg_idx].content.len());
            aged += 1;
        }

        if aged > 0 {
            debug!(
                "[tool_aging] 老化 {} 个 tool 结果，释放 {} 字节",
                aged, freed
            );
        }

        (aged, freed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tool_executor::ToolExecutor;

    fn make_tool_msg(content: &str, call_id: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            content: content.to_string(),
            name: None,
            tool_calls: None,
            tool_call_id: Some(call_id.to_string()),
            reasoning_content: None,
        }
    }

    fn make_system_msg() -> ChatMessage {
        ChatMessage {
            role: "system".to_string(),
            content: "sys".to_string(),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
        }
    }

    fn default_settings() -> ToolResultAgingSettings {
        ToolResultAgingSettings {
            enabled: true,
            keep_full: 3,
            try_microtool: 3,
            compress_threshold: 200,
        }
    }

    #[test]
    fn test_aging_keeps_recent_full() {
        let aging = ToolResultAging::new(&default_settings());
        let executor = RwLock::new(ToolExecutor::new());

        let mut msgs = vec![make_system_msg()];
        for i in 0..5 {
            msgs.push(make_tool_msg(&"x".repeat(500), &format!("call_{}", i)));
        }

        let (aged, _) = aging.age_tool_results(&mut msgs, &executor);

        // keep_full=3: 保留最新的 3 条（call_4/3/2），最旧的 2 条被压缩（call_0/1）
        // call_0/1 rev_position 4/3 < microtool_end(6) → [旧结果] 前缀
        assert_eq!(aged, 2, "should age 2 oldest results");

        let contents: Vec<&str> = msgs.iter().filter(|m| m.role == "tool").map(|m| m.content.as_str()).collect();
        assert!(contents[0].starts_with("[旧结果"), "call_0 oldest should be compressed");
        assert!(contents[1].starts_with("[旧结果"), "call_1 oldest should be compressed");
        assert!(contents[2].starts_with("x"), "call_2 should remain full (recent)");
        assert!(contents[3].starts_with("x"), "call_3 should remain full (recent)");
        assert!(contents[4].starts_with("x"), "call_4 should remain full (recent)");
    }

    #[test]
    fn test_aging_microtool_reference() {
        let aging = ToolResultAging::new(&ToolResultAgingSettings {
            enabled: true,
            keep_full: 1,
            try_microtool: 2,
            compress_threshold: 50,
        });
        let executor = RwLock::new(ToolExecutor::new());

        let mut msgs = vec![make_system_msg()];
        for i in 0..4 {
            msgs.push(make_tool_msg(&"y".repeat(200), &format!("call_{}", i)));
        }

        let (aged, _) = aging.age_tool_results(&mut msgs, &executor);

        // total=4, keep_full=1, microtool_end=3
        // rev_positions: call_0=3, call_1=2, call_2=1, call_3=0
        // call_3(rev=0) < keep_full(1) → kept full
        // call_2(rev=1) < keep_full? NO, < microtool_end(3)? YES → microtool → [旧结果]
        // call_1(rev=2) < keep_full? NO, < microtool_end(3)? YES → microtool → [旧结果]
        // call_0(rev=3) < keep_full? NO, < microtool_end(3)? NO → oldest → [历史结果]
        assert_eq!(aged, 3);

        let contents: Vec<&str> = msgs.iter().filter(|m| m.role == "tool").map(|m| m.content.as_str()).collect();
        // call_0 (idx 0 in msgs, oldest): rev=3 >= microtool_end → [历史结果]
        assert!(contents[0].starts_with("[历史结果"), "call_0 oldest should be brief summary");
        // call_1 (idx 1): rev=2 < microtool_end → [旧结果]
        assert!(contents[1].starts_with("[旧结果"), "call_1 should be in microtool range");
        // call_2 (idx 2): rev=1 < microtool_end → [旧结果]
        assert!(contents[2].starts_with("[旧结果"), "call_2 should be in microtool range");
        // call_3 (idx 3, newest): rev=0 < keep_full → kept full
        assert!(contents[3].starts_with("y"), "call_3 newest should be full");
    }

    #[test]
    fn test_aging_skips_small_results() {
        let aging = ToolResultAging::new(&ToolResultAgingSettings {
            enabled: true,
            keep_full: 1,
            try_microtool: 2,
            compress_threshold: 500, // 小于 500 字节的不处理
        });
        let executor = RwLock::new(ToolExecutor::new());

        let mut msgs = vec![make_system_msg()];
        for i in 0..4 {
            msgs.push(make_tool_msg(&"small".to_string(), &format!("call_{}", i)));
        }

        let (aged, _) = aging.age_tool_results(&mut msgs, &executor);
        assert_eq!(aged, 0, "small results should not be aged");
    }

    #[test]
    fn test_aging_frees_bytes() {
        let aging = ToolResultAging::new(&default_settings());
        let executor = RwLock::new(ToolExecutor::new());

        let mut msgs = vec![make_system_msg()];
        for i in 0..4 {
            msgs.push(make_tool_msg(&"x".repeat(500), &format!("call_{}", i)));
        }

        let (_, freed) = aging.age_tool_results(&mut msgs, &executor);
        assert!(freed > 0, "should free bytes from aging");
    }
}
