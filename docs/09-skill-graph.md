# 9. 技能图谱系统

> 基于 JSON-LD 的技能知识图谱，支持 5W2H 描述、技能发现、进化、冲突检测和自举学习

## 模块架构

**源文件目录**: `src/skill_graph/`（12 个模块）

```mermaid
graph TB
    subgraph 输入源
        MD["Markdown Skill"]
        LLM_IN["LLM 自然语言"]
        MCP_IN["MCP 工具"]
        BOOT_IN["任务执行/错误恢复/<br/>用户反馈/代码审查"]
    end

    subgraph 创建层
        SC["SkillCreator<br/>LLM 技能创建<br/>MD→JSON-LD 转换"]
        MCP_INT["MCPIntegration<br/>MCP 工具同步"]
        BSE["BootstrapEngine<br/>自举学习"]
    end

    subgraph 存储层
        SGS["SkillGraphStore<br/>技能图谱存储<br/>L0+L2 双层"]
        IDX["PreAggregatedIndex<br/>预聚合索引"]
    end

    subgraph 运行时引擎
        SDE["SkillDiscoveryEngine<br/>5W2H 匹配 + 向量检索"]
        SEE["SkillEvolutionEngine<br/>使用追踪 + 进化建议"]
        SEC["SecurityEngine<br/>信任等级 + 签名校验"]
        CFE["ConflictDetectionEngine<br/>6种冲突检测"]
        QE["QueryEngine<br/>查询模板"]
    end

    MD --> SC
    LLM_IN --> SC
    MCP_IN --> MCP_INT
    BOOT_IN --> BSE
    SC --> SGS
    MCP_INT --> SGS
    BSE --> SGS
    SGS --> IDX
    IDX --> SDE
    SGS --> SEE
    SGS --> SEC
    SGS --> CFE
    SGS --> QE
```

## 模块文件清单

| 文件 | 组件 | 说明 |
|------|------|------|
| `types.rs` | SkillGraphNode, SkillNodeType, SkillLink, Skill5W2H | 核心类型定义 |
| `graph_store.rs` | SkillGraphStore | 技能图谱存储（L0 + L2） |
| `index.rs` | PreAggregatedIndex | 预聚合索引 |
| `discovery.rs` | SkillDiscoveryEngine | 5W2H 技能发现引擎 |
| `evolution.rs` | SkillEvolutionEngine | 技能进化引擎 |
| `conflict.rs` | ConflictDetectionEngine | 冲突检测引擎 |
| `security.rs` | SecurityEngine | 安全引擎 |
| `skill_creator.rs` | SkillCreator | LLM 技能创建 |
| `bootstrap.rs` | BootstrapEngine | 自举学习 |
| `mcp_integration.rs` | MCPIntegration | MCP 工具同步 |
| `query_templates.rs` | QueryEngine | 查询模板 |

## 核心类型

### SkillGraphNode — 技能节点

```mermaid
classDiagram
    class SkillGraphNode {
        +String skill_iri
        +String name
        +String description
        +String version
        +SkillNodeType node_type
        +Skill5W2H w2h
        +Vec~SkillLink~ links
        +SkillGraphMeta graph_meta
        +Option~SkillContent~ content
        +Option~SkillSecurityInfo~ security_info
        +StorageTier storage_tier
        +to_json_ld() Value
    }

    class SkillNodeType {
        <<enumeration>>
        Atomic
        Composite
        MOC
        KnowledgeFragment
        MCPTool
        Bootstrap
    }

    class Skill5W2H {
        +String what
        +String why
        +String who
        +String when
        +String where
        +String how
        +Option~String~ how_much
    }

    class SkillLink {
        +String target_iri
        +SkillLinkType link_type
        +Option~String~ description
        +f64 confidence
    }

    class SkillLinkType {
        <<enumeration>>
        Prerequisite
        Composition
        Related
        Alternative
        Extends
        Generalization
    }

    SkillGraphNode --> SkillNodeType
    SkillGraphNode --> Skill5W2H
    SkillGraphNode --> SkillLink
    SkillLink --> SkillLinkType
```

### 存储层级

| 层级 | 类型 | 说明 |
|------|------|------|
| `L0Permanent` | sled | 永久存储，核心技能 |
| `L1Session` | 内存 | 会话级临时技能 |
| `L2Blackboard` | Oxigraph | 共享黑板，跨 Agent 可见 |
| `L3Projection` | SPARQL | 按需投影 |

## 引擎详解

### SkillDiscoveryEngine — 技能发现

基于 5W2H 维度匹配和向量检索的技能发现：

```mermaid
flowchart LR
    TASK["任务描述"] --> W2H["5W2H 解析"]
    W2H --> MATCH["5W2H 维度匹配<br/>what/why/who/when/where"]
    W2H --> VEC["向量相似度检索"]
    MATCH --> MERGE["结果融合"]
    VEC --> MERGE
    MERGE --> RANK["置信度排序"]
    RANK --> TOP["返回 Top-K 技能"]
```

**文件**: `src/skill_graph/discovery.rs`

### SkillEvolutionEngine — 技能进化

追踪技能使用情况，生成进化建议：

**文件**: `src/skill_graph/evolution.rs`

| 进化建议类型 | 说明 |
|-------------|------|
| `AddLink` | 添加新的技能关联 |
| `UpdateSuccessRate` | 更新成功率 |
| `CreateFragment` | 创建知识碎片 |
| `Deprecate` | 标记废弃 |
| `Merge` | 合并相似技能 |
| `Split` | 拆分过大的技能 |

### ConflictDetectionEngine — 冲突检测

**文件**: `src/skill_graph/conflict.rs`

6 种冲突类型：

| 冲突类型 | 说明 |
|---------|------|
| `Resource` | 资源竞争冲突 |
| `Dependency` | 依赖版本冲突 |
| `Permission` | 权限冲突 |
| `Semantic` | 语义定义冲突 |
| `Temporal` | 时序冲突 |
| `Version` | 版本冲突 |

### SecurityEngine — 安全引擎

**文件**: `src/skill_graph/security.rs`

```mermaid
flowchart TD
    CALL["技能调用请求"] --> TRUST["检查信任等级"]
    TRUST --> PERM["检查权限列表"]
    PERM --> SIG["校验数字签名<br/>(Ed25519)"]
    SIG --> RISK["评估风险分数"]
    RISK --> DECISION{"安全决策"}
    DECISION -->|"Allow"| EXEC["执行技能"]
    DECISION -->|"Deny"| REJECT["拒绝执行"]
    DECISION -->|"AskUser"| PROMPT["提示用户确认"]
```

### SkillCreator — LLM 技能创建

**文件**: `src/skill_graph/skill_creator.rs`

支持两种创建模式：

1. **自然语言创建**：用户描述需求 → LLM 生成 JSON-LD Skill 定义
2. **Markdown 转换**：读取 skill.md → LLM 转换为 JSON-LD 格式

### BootstrapEngine — 自举学习

**文件**: `src/skill_graph/bootstrap.rs`

从运行时经验中自动学习新技能：

| 学习来源 | 说明 |
|---------|------|
| 任务执行 | 成功执行的任务模式 |
| 错误恢复 | 修复错误的策略 |
| 用户反馈 | 用户显式指导 |
| 代码审查 | 代码改进建议 |
| 知识抽取 | 从文档中提取 |

**操作类型**：
- `Learn` — 创建新技能或增强现有技能
- `Reduce` — 简化过于复杂的技能

### MCPIntegration — MCP 工具同步

**文件**: `src/skill_graph/mcp_integration.rs`

将 MCP 工具自动同步为技能图谱中的技能节点。

## MOC 导航

MOC（Map of Content）节点作为技能图谱的导航入口：

```mermaid
graph TD
    MOC["MOC: 编程技能"] --> S1["Rust 开发"]
    MOC --> S2["Python 开发"]
    MOC --> S3["Web 开发"]
    S1 --> S1_1["Rust 异步编程"]
    S1 --> S1_2["Rust 宏编程"]
    S2 --> S2_1["Python 数据分析"]
    S3 --> S3_1["React 开发"]
    S3 --> S3_2["Node.js 开发"]
    S1_1 -.->|"Related"| S3_2
```

## 与知识图谱的关系

技能图谱和知识图谱是互补的双层架构：

```mermaid
graph LR
    subgraph 技能图谱
        SKILL["技能节点<br/>How/Process"]
    end

    subgraph 知识图谱
        ENTITY["知识实体<br/>What/Concept"]
    end

    SKILL -->|"HasSkill"| ENTITY
    ENTITY -->|"ApplicableIn"| SKILL
    SKILL -->|"RelatedTo"| ENTITY
```

| 维度 | 技能图谱 | 知识图谱 |
|------|---------|---------|
| 存储 | L0 sled + L2 Oxigraph | Oxigraph Memory（`Arc<Mutex>`） |
| 命名图 | `graph:skill` | `graph:world` / `graph:code` |
| 描述 | 5W2H 结构化 | RDF Quads |
| 发现 | 5W2H 匹配 + 向量检索 | SPARQL + 模糊搜索 |
| 进化 | 使用追踪 + 进化建议 | 增量更新（SHA256） |
