use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemPromptRegion {
    RoleDefinition,
    FiveW2HConstraints,
    EmphasizedConstraints,
    OutputFormat,
    OutputManagement,
    Tools,
    ExtractionPrompt,
}

impl SystemPromptRegion {
    pub fn order(&self) -> usize {
        match self {
            Self::RoleDefinition => 1,
            Self::FiveW2HConstraints => 2,
            Self::EmphasizedConstraints => 3,
            Self::OutputFormat => 4,
            Self::OutputManagement => 5,
            Self::Tools => 6,
            Self::ExtractionPrompt => 7,
        }
    }

    pub fn header(&self) -> &'static str {
        match self {
            Self::RoleDefinition => "# 角色",
            Self::FiveW2HConstraints => "# 任务约束",
            Self::EmphasizedConstraints => "# 重要约束",
            Self::OutputFormat => "# 输出格式",
            Self::OutputManagement => "# 输出管理",
            Self::Tools => "# 工具",
            Self::ExtractionPrompt => "# 强调内容",
        }
    }
}

pub const OUTPUT_FORMAT_SIMPLE: &str = r#"返回 JSON: {"content": "...", "summary": "...", "action": "tool_call|finish"}
- summary: ≤50字摘要
- action: tool_call(调用工具) 或 finish(任务完成)"#;

pub const OUTPUT_FORMAT_FULL: &str = r#"返回 JSON: {"content": "...", "summary": "...", "action": "tool_call|finish|continue", "emphasis": []}
- content: 回复内容
- summary: ≤50字摘要
- action: tool_call(调用工具) / finish(任务完成) / continue(继续思考)
- emphasis: 识别的重要约束（数组）"#;

/// Instructions for output management — injected into system prompt between OutputFormat and Tools regions.
/// Tells the LLM how to handle large command output proactively.
pub const OUTPUT_MANAGEMENT: &str = r#"📋 输出管理 — 所有工具（尤其是 bash）必须遵守：

1. **大输出必须过滤**：可能返回 >100 行的命令，必须加 | head -N 或 | grep 关键字 限制输出量
2. **精确搜索优先**：grep / find 等必须指定路径范围，不得扫描整个工作区
3. **按需确认**：只需确认结果是否存在时，使用 | grep -c 或 | wc -l 而非查看全部内容
4. **截断感知**：超过 16KB 的输出会被静默截断，超过 2KB 的结果会被摘要化并附带 IRI 存档
   - 若看到「output truncated」标记或「[已存档]」标签 → 说明输出太大，请缩小范围重新搜索
   - 如需查看完整结果，可使用 read_full_result_* 工具按需读取"#;

pub fn build_five_w2h_section(snapshot: &crate::core::five_w2h::Task5W2H) -> String {
    let mut lines = Vec::new();
    
    lines.push(format!("- 目标: {}", snapshot.what));
    
    lines.push(format!("- 原因: {}", snapshot.why.description));
    if !snapshot.why.success_criteria.is_empty() {
        lines.push(format!("- 成功标准: {}", snapshot.why.success_criteria.join(", ")));
    }
    
    if let Some(ref who) = snapshot.who {
        if let Some(ref requestor) = who.requestor {
            lines.push(format!("- 请求者: {}", requestor));
        }
        if let Some(ref access_level) = who.access_level {
            lines.push(format!("- 访问级别: {:?}", access_level));
        }
        if !who.assignees.is_empty() {
            lines.push(format!("- 执行者: {}", who.assignees.join(", ")));
        }
    }
    
    if let Some(ref when) = snapshot.when {
        if let Some(ref deadline) = when.deadline {
            lines.push(format!("- 截止时间: {}", deadline));
        }
    }
    
    if let Some(ref where_) = snapshot.where_ {
        if let Some(ref env) = where_.execution_environment {
            lines.push(format!("- 执行环境: {}", env));
        }
        if !where_.data_sources.is_empty() {
            lines.push(format!("- 数据源: {}", where_.data_sources.join(", ")));
        }
    }
    
    if let Some(ref how) = snapshot.how {
        if !how.forbidden_tools.is_empty() {
            lines.push(format!("- 禁用工具: {}", how.forbidden_tools.join(", ")));
        }
        if let Some(ref steps) = how.required_steps {
            lines.push(format!("- 要求步骤: {}", steps));
        }
        if !how.preferred_skills.is_empty() {
            lines.push(format!("- 所需技能: {}", how.preferred_skills.join(", ")));
        }
    }
    
    if let Some(ref how_much) = snapshot.how_much {
        if let Some(ref budget) = how_much.token_budget {
            lines.push(format!("- Token预算: {}", budget));
        }
        if let Some(ref cycles) = how_much.max_pdca_cycles {
            lines.push(format!("- 最大循环: {}", cycles));
        }
    }
    
    lines.join("\n")
}

pub struct ToolRegionContent {
    pub builtin_tools: String,
    pub dynamic_tools: String,
}

impl ToolRegionContent {
    pub fn new() -> Self {
        Self {
            builtin_tools: String::new(),
            dynamic_tools: String::new(),
        }
    }

    pub fn with_builtin(mut self, tools: &str) -> Self {
        self.builtin_tools = tools.to_string();
        self
    }

    pub fn with_dynamic(mut self, tools: &str) -> Self {
        self.dynamic_tools = tools.to_string();
        self
    }

    pub fn build(&self) -> String {
        let mut parts = Vec::new();
        
        if !self.builtin_tools.is_empty() {
            parts.push(format!("## 内置工具（固定）\n{}", self.builtin_tools));
        }
        
        if !self.dynamic_tools.is_empty() {
            parts.push(format!("## 动态工具（按需调整）\n{}", self.dynamic_tools));
        }
        
        parts.join("\n\n")
    }
}

impl Default for ToolRegionContent {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SystemPromptBuilder {
    regions: HashMap<SystemPromptRegion, String>,
}

impl SystemPromptBuilder {
    pub fn new() -> Self {
        Self {
            regions: HashMap::new(),
        }
    }

    pub fn set_region(&mut self, region: SystemPromptRegion, content: String) {
        self.regions.insert(region, content);
    }

    pub fn get_region(&self, region: &SystemPromptRegion) -> Option<&String> {
        self.regions.get(region)
    }

    pub fn clear_region(&mut self, region: &SystemPromptRegion) {
        self.regions.remove(region);
    }

    pub fn build(&self) -> String {
        let mut ordered_regions: Vec<(&SystemPromptRegion, &String)> = 
            self.regions.iter().collect();
        ordered_regions.sort_by_key(|(r, _)| r.order());

        let mut parts = Vec::new();
        for (region, content) in ordered_regions {
            if !content.is_empty() {
                parts.push(format!("{}\n\n{}", region.header(), content));
            }
        }
        parts.join("\n\n---\n\n")
    }

    pub fn build_with_emphasis(&self, emphasis_items: &[String]) -> String {
        let mut builder = self.clone();
        
        if !emphasis_items.is_empty() {
            let emphasis_content = emphasis_items
                .iter()
                .map(|e| format!("- {}", e))
                .collect::<Vec<_>>()
                .join("\n");
            builder.set_region(SystemPromptRegion::EmphasizedConstraints, emphasis_content);
        }
        
        builder.build()
    }
}

impl Default for SystemPromptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for SystemPromptBuilder {
    fn clone(&self) -> Self {
        Self {
            regions: self.regions.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_region_order() {
        assert!(SystemPromptRegion::RoleDefinition.order() < SystemPromptRegion::FiveW2HConstraints.order());
        assert!(SystemPromptRegion::FiveW2HConstraints.order() < SystemPromptRegion::EmphasizedConstraints.order());
        assert!(SystemPromptRegion::EmphasizedConstraints.order() < SystemPromptRegion::OutputFormat.order());
        assert!(SystemPromptRegion::OutputFormat.order() < SystemPromptRegion::Tools.order());
        assert!(SystemPromptRegion::Tools.order() < SystemPromptRegion::ExtractionPrompt.order());
    }

    #[test]
    fn test_build_system_prompt() {
        let mut builder = SystemPromptBuilder::new();
        builder.set_region(SystemPromptRegion::RoleDefinition, "你是计划Agent".to_string());
        builder.set_region(SystemPromptRegion::OutputFormat, "输出JSON格式".to_string());
        
        let result = builder.build();
        assert!(result.contains("# 角色"));
        assert!(result.contains("# 输出格式"));
        assert!(result.contains("你是计划Agent"));
    }

    #[test]
    fn test_build_with_emphasis() {
        let mut builder = SystemPromptBuilder::new();
        builder.set_region(SystemPromptRegion::RoleDefinition, "你是计划Agent".to_string());
        
        let emphasis = vec!["必须使用异步方式".to_string(), "注意错误处理".to_string()];
        let result = builder.build_with_emphasis(&emphasis);
        
        assert!(result.contains("重要约束"));
        assert!(result.contains("必须使用异步方式"));
        assert!(result.contains("注意错误处理"));
    }

    #[test]
    fn test_tool_region_content() {
        let tool_content = ToolRegionContent::new()
            .with_builtin("file_read: 读取文件\nfile_write: 写入文件")
            .with_dynamic("http_request: HTTP请求\ncode_execute: 执行代码");
        
        let result = tool_content.build();
        assert!(result.contains("内置工具（固定）"));
        assert!(result.contains("动态工具（按需调整）"));
        assert!(result.contains("file_read"));
        assert!(result.contains("http_request"));
    }

    #[test]
    fn test_build_with_tools() {
        let mut builder = SystemPromptBuilder::new();
        builder.set_region(SystemPromptRegion::RoleDefinition, "你是执行Agent".to_string());
        
        let tool_content = ToolRegionContent::new()
            .with_builtin("file_read: 读取文件")
            .with_dynamic("custom_tool: 自定义工具")
            .build();
        builder.set_region(SystemPromptRegion::Tools, tool_content);
        
        let result = builder.build();
        assert!(result.contains("# 工具"));
        assert!(result.contains("内置工具（固定）"));
        assert!(result.contains("动态工具（按需调整）"));
    }
}
