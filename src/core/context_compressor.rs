use std::collections::VecDeque;
use serde::{Deserialize, Serialize};
use crate::config::settings::{ToolResultCompressorSettings, ContextWindowSettings};
use crate::gateway::unified_gateway::ChatMessage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultEntry {
    pub turn: u32,
    pub tool_name: String,
    pub content: String,
    pub is_compressed: bool,
}

pub struct ToolResultCompressor {
    enabled: bool,
    max_full_results: usize,
    max_summary_length: usize,
    compression_trigger: usize,
    results: VecDeque<ToolResultEntry>,
}

impl ToolResultCompressor {
    pub fn new(settings: &ToolResultCompressorSettings) -> Self {
        Self {
            enabled: settings.enabled,
            max_full_results: settings.max_full_results,
            max_summary_length: settings.max_summary_length,
            compression_trigger: settings.compression_trigger,
            results: VecDeque::new(),
        }
    }
    
    pub fn add_result(&mut self, turn: u32, tool_name: &str, content: &str) {
        let entry = ToolResultEntry {
            turn,
            tool_name: tool_name.to_string(),
            content: content.to_string(),
            is_compressed: false,
        };
        self.results.push_back(entry);
        
        if self.results.len() >= self.compression_trigger {
            self.compress_old_results();
        }
    }
    
    fn compress_old_results(&mut self) {
        if self.results.len() <= self.max_full_results {
            return;
        }
        
        let to_compress = self.results.len() - self.max_full_results;
        let summaries: Vec<(usize, String)> = self.results.iter().take(to_compress)
            .enumerate()
            .filter(|(_, entry)| !entry.is_compressed && entry.content.len() > self.max_summary_length)
            .map(|(i, entry)| (i, self.summarize_content(&entry.content)))
            .collect();
        
        for (i, summary) in summaries {
            if let Some(entry) = self.results.get_mut(i) {
                entry.content = summary;
                entry.is_compressed = true;
            }
        }
    }
    
    fn summarize_content(&self, content: &str) -> String {
        if content.len() <= self.max_summary_length {
            return content.to_string();
        }
        
        let lines: Vec<&str> = content.lines().take(5).collect();
        let preview = if lines.len() > 3 {
            lines[..3].join("\n")
        } else {
            lines.join("\n")
        };
        
        format!("[摘要 {}字节] {}... (共 {} 字符)", 
            self.max_summary_length, 
            preview,
            content.len()
        )
    }
    
    pub fn get_results(&self) -> &VecDeque<ToolResultEntry> {
        &self.results
    }
    
    pub fn clear(&mut self) {
        self.results.clear();
    }
    
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

pub struct ContextWindowManager {
    max_messages: usize,
    max_tokens: usize,
    compression_ratio: f32,
    preserve_recent: usize,
}

impl ContextWindowManager {
    pub fn new(settings: &ContextWindowSettings) -> Self {
        Self {
            max_messages: settings.max_messages,
            max_tokens: settings.max_tokens,
            compression_ratio: settings.compression_ratio,
            preserve_recent: settings.preserve_recent,
        }
    }
    
    pub fn should_compress(&self, message_count: usize) -> bool {
        message_count > self.max_messages
    }
    
    pub fn compress_messages(&self, messages: &[ChatMessage]) -> (Vec<ChatMessage>, String) {
        if messages.len() <= self.max_messages {
            return (messages.to_vec(), String::new());
        }
        
        let system_msg = messages.first().filter(|m| m.role == "system").cloned();
        let recent_start = messages.len().saturating_sub(self.preserve_recent);
        let recent: Vec<_> = messages[recent_start..].to_vec();
        
        let middle_start = if system_msg.is_some() { 1 } else { 0 };
        let middle: Vec<_> = messages[middle_start..recent_start].to_vec();
        
        let summary = self.summarize_middle_messages(&middle);
        
        let mut compressed = Vec::new();
        if let Some(sys) = system_msg {
            compressed.push(sys);
        }
        
        if !summary.is_empty() {
            compressed.push(ChatMessage {
                role: "user".to_string(),
                content: format!("[历史摘要] {}", summary),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
            });
        }
        
        compressed.extend(recent);
        (compressed, summary)
    }
    
    fn summarize_middle_messages(&self, messages: &[ChatMessage]) -> String {
        let mut tool_calls = Vec::new();
        let mut summaries = Vec::new();
        let mut errors = Vec::new();
        
        for msg in messages {
            match msg.role.as_str() {
                "assistant" => {
                    if let Some(ref tool_calls_data) = msg.tool_calls {
                        for tc in tool_calls_data {
                            tool_calls.push(tc.function.name.clone());
                        }
                    }
                    if msg.content.len() > 50 && msg.content.len() < 200 {
                        summaries.push(msg.content.clone());
                    }
                }
                "tool" => {
                    if msg.content.contains("error") || msg.content.contains("Error") {
                        errors.push(msg.content.chars().take(100).collect::<String>());
                    }
                }
                _ => {}
            }
        }
        
        let mut parts = Vec::new();
        
        if !tool_calls.is_empty() {
            let unique_tools: std::collections::HashSet<_> = tool_calls.into_iter().collect();
            parts.push(format!("调用工具: {}", unique_tools.into_iter().collect::<Vec<_>>().join(", ")));
        }
        
        if !errors.is_empty() {
            parts.push(format!("错误: {} 个", errors.len()));
        }
        
        if !summaries.is_empty() {
            parts.push(format!("关键内容: {}", summaries.join("; ")));
        }
        
        parts.join(" | ")
    }
    
    pub fn max_messages(&self) -> usize {
        self.max_messages
    }
    
    pub fn max_tokens(&self) -> usize {
        self.max_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    fn default_settings() -> ToolResultCompressorSettings {
        ToolResultCompressorSettings {
            enabled: true,
            max_full_results: 2,
            max_summary_length: 200,
            compression_trigger: 5,
        }
    }
    
    fn default_context_settings() -> ContextWindowSettings {
        ContextWindowSettings {
            max_messages: 15,
            max_tokens: 16000,
            compression_ratio: 0.3,
            preserve_recent: 4,
        }
    }
    
    #[test]
    fn test_compressor_add_result() {
        let mut compressor = ToolResultCompressor::new(&default_settings());
        
        compressor.add_result(1, "file_read", "test content");
        assert_eq!(compressor.get_results().len(), 1);
    }
    
    #[test]
    fn test_compressor_compress() {
        let mut compressor = ToolResultCompressor::new(&default_settings());
        
        let long_content = "x".repeat(500);
        
        for i in 1..=6 {
            compressor.add_result(i, "file_read", &long_content);
        }
        
        let results = compressor.get_results();
        assert!(results.front().unwrap().is_compressed);
        assert!(!results.back().unwrap().is_compressed);
    }
    
    #[test]
    fn test_context_window_should_compress() {
        let manager = ContextWindowManager::new(&default_context_settings());
        
        assert!(!manager.should_compress(10));
        assert!(manager.should_compress(20));
    }
}
