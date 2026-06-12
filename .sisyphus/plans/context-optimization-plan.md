# 上下文管理优化方案

## P0 — 关键缺陷修复

### P0.1 流式路径添加压缩机制

**当前问题:**
`execute_streaming_inner()` (utils.rs:605) 中的 `running_messages` 在 ReAct 循环中从不压缩。每轮完整克隆后发送给 LLM，max_turns=10 时最多可达 40+ 条消息。

**设计方案:**
在流式路径的工具调用循环末尾（`turn += 1` 之后），插入 ContextWindowManager 压缩检查：

```
每轮循环结束时:
  if turn >= 压缩检查间隔(3轮):
    使用已有的 context_window_manager 压缩 running_messages
    压缩后的消息替换 running_messages
    保留 system 消息不变
```

**实现要点:**
- 复用已有的 `self.context_window_manager`（Arc<Mutex<ContextWindowManager>>）
- 压缩间隔设为每 3 轮一次（避免每轮都压缩）
- 压缩逻辑与 `execution.rs` 中的非流式路径一致：system 消息保留、adjust_boundary_for_tool_calls、remove_orphaned_tool_messages
- 流式路径的 `handle_turn_streaming()` 需要额外判断是否需要将整个 `running_messages` 发给 LLM，还是只发新的消息

**涉及文件:** `src/core/agent_runner/utils.rs`

**影响:**
- Token 消耗降低 ~40%（最坏情况）
- 对用户无感知（流式输出不受影响）
- 风险低（压缩逻辑完全复用已有代码）

---

### P0.2 ToolResultCompressor 死代码修复 (Option B: 真正联通)

**当前问题:**
`compressor.add_result()` 在 execution.rs:1311 被调用，但 `get_results()` 从未被读取。ToolResultCompressor 存储了压缩后的结果，但从不写回 `messages`。

**设计方案 (选 B — 真正联通):**

1. 给 ToolResultCompressor 添加方法 `compress_old_tool_messages(messages: &mut Vec<ChatMessage>)`：
   - 扫描 messages 中 role="tool" 的消息
   - 如果 content 长度 > max_summary_length
   - 且该 tool 已被 compressor 标记为 compressed
   - 则替换为 compressor 中的摘要版本

2. 在 execution.rs 的 `add_result()` 调用后立即调用此方法

```
// execution.rs 中的改动:
compressor.add_result(turn, name, &result_str);
compressor.compress_old_tool_messages(&mut messages);  // 新增
```

3. ToolResultCompressor 新增方法：
```rust
pub fn compress_old_tool_messages(&self, messages: &mut Vec<ChatMessage>) {
    if !self.enabled { return; }
    // 对每个已被压缩的 entry，找到 messages 中对应的 tool 消息并替换
    for entry in &self.results {
        if !entry.is_compressed { continue; }
        for msg in messages.iter_mut() {
            if msg.role == "tool" && msg.content == entry.content {
                // 已经被压缩过或者不匹配
            }
        }
    }
}
```

这个方案的问题是：压缩器内部压缩后 content 变了，无法通过 content 匹配。更简单的方案：

**简化的正确方案:**
直接在 `compress_old_results()` 中，在压缩 compressor 内部 entry 的同时，也标记这些 entry 的索引位置。然后 `compress_old_tool_messages()` 根据这些索引位置去匹配 messages 中对应的 tool 消息（通过 turn + tool_name）。

实际上最简单的实现：**每轮在 add_result 后，直接对 messages 中的 tool 消息内容做硬截断**  — 超过 max_summary_length 的直接替换为摘要，不依赖 compressor 的内部状态匹配。

```rust
pub fn compress_tool_messages(&self, messages: &mut Vec<ChatMessage>) {
    let max_len = self.max_summary_length;
    for msg in messages.iter_mut() {
        if msg.role == "tool" && msg.content.len() > max_len {
            let preview: String = msg.content.chars().take(max_len).collect();
            msg.content = format!("[已压缩 {}字节] {}...", msg.content.len(), preview);
        }
    }
}
```

**涉及文件:** `src/core/context_compressor.rs`, `src/core/agent_runner/execution.rs`

---

### P0.3 压缩阈值改为基于 Token 估算

**当前问题:**
`execution.rs:676` 硬编码 `max_context_messages = 30`，基于消息条数而非 token 消耗。30 条短消息可能 token 很少，7-8 条长消息可能已经超限。

**设计方案:**

1. 给 ContextWindowManager 添加 token 估算方法：
```rust
pub fn estimate_tokens(messages: &[ChatMessage]) -> usize {
    messages.iter().map(|m| {
        m.content.len() / 3  // 中英文混合约 3-4 字符/token
        + m.role.len() / 3
        + m.tool_calls.as_ref().map(|c| ...).unwrap_or(0)
    }).sum()
}
```

2. 修改 `should_compress` 方法：
```rust
pub fn should_compress(&self, message_count: usize, messages: &[ChatMessage]) -> bool {
    if message_count > self.max_messages { return true; }
    if Self::estimate_tokens(messages) > self.max_tokens { return true; }
    false
}
```

3. 在 execution.rs 中，调用 `should_compress` 时传入消息切片和 token 估算值

4. ContextWindowManager 配置中已有 `max_tokens` 字段（context_compressor.rs:101），设置默认值为 16000

**涉及文件:** `src/core/context_compressor.rs`, `src/core/agent_runner/execution.rs`

---

## P1 — 重要优化

### P1.1 Emphasis 加载前缀扫描

**当前问题:**
`load_emphasis_from_l0()` (utils.rs:262) 每次调用都 `search_by_tags(["emphasis"])` 全量搜索所有 emphasis 节点，然后逐条过滤 task_iri。

**设计方案:**
- 给 L0Store 添加 `scan_iri_prefix(prefix: &str)` 方法，利用 Sled 的键范围扫描
- 在 save_emphasis_to_l0 时使用固定的 IRI 模式：`iri://emphasis/{task_iri}/{uuid}`
- 在 load_emphasis_from_l0 时使用 `scan_iri_prefix("iri://emphasis/{task_iri}")` 按前缀扫描

### P1.2 L1 summary_chain 动态截断

**当前问题:**
`get_summary_chain_with_iris(50, 200)` 在 execution.rs:445 固定 max_turns=50, summary_length=200。

**设计方案:**
- 基于模型 context window 动态调整参数
- 或改为配置化，从 ContextWindowSettings 读取
- 或保守减少：max_turns=20, summary_length=100

### P1.3 消息每轮完整 clone

**当前问题:**
`messages.clone()` 在 execution.rs:799 每轮调用。

**设计方案:**
- 使用 `Arc<Vec<ChatMessage>>` 作为中间传递，仅在修改时 clone
- 或推迟 clone：LLM 调用可能是只读的（取决于 gateway 实现）

### P1.4 5W2H 按角色去重

**设计方案:**
- 在 `gather_context_data()` 中按角色裁剪 5W2H 维度
- PA: what, why, deadline, env
- DA: what, required_steps
- CA: 完整 7 维度
- AA: 不需要 5W2H（只用 CA 结论）

---

## P2 — 性能优化

### P2.1 SystemPromptBuilder 原地修改

**当前问题:**
`build_with_emphasis()` (system_prompt.rs:267) 克隆整个 builder。

**设计方案:**
添加 `set_emphasis(emphasis_items)` 方法，直接修改 regions HashMap，不克隆整个 builder：
```rust
pub fn set_emphasis(&mut self, emphasis_items: &[String]) {
    let content = emphasis_items.iter().map(|e| format!("- {}", e)).join("\n");
    self.set_region(SystemPromptRegion::EmphasizedConstraints, content);
}
```
在调用处改用两步：`builder.set_emphasis(items); builder.build();`

### P2.2 max_items 未生效

**当前问题:**
配置中 `max_items: 50` 在 `save_emphasis_to_l0()` 中未截断。

**设计方案:**
在 `save_emphasis_to_l0()` 入口处截断：
```rust
let items: Vec<_> = emphasis_items.iter().take(self.emphasis_config.as_ref().map_or(50, |c| c.max_items)).collect();
```

### P2.3-P2.5 缓存优化

- adjust_boundary_for_tool_calls 缓存已知 tool_call_id
- PromptLoader 添加 mtime 缓存
- TemplateManager 改为惰性加载

---

## P3 — 长期架构

### P3.1 相关性上下文裁剪 (不做，仅做架构设计参考)

**设计方向:**
- 给 ChatMessage 添加 priority/relevance 字段
- 消息注入时标记：用户指令=高, 工具失败=高, 中间结果=低
- 压缩时按优先级保留，优先丢弃低优先级消息
