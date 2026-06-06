use super::types::{OntologyTerm, OntologyTermType};
use std::sync::OnceLock;

static BUILT_IN_ONTOLOGY: OnceLock<Vec<OntologyTerm>> = OnceLock::new();

fn core(s: &str) -> String { format!("https://agentos.ontology/core/{}", s) }
fn eng(s: &str)  -> String { format!("https://agentos.ontology/eng/{}", s) }
fn code(s: &str) -> String { format!("https://agentos.ontology/code/{}", s) }
fn biz(s: &str)  -> String { format!("https://agentos.ontology/biz/{}", s) }

pub struct OntologyManager {
    domain_terms: Vec<OntologyTerm>,
}

impl OntologyManager {
    pub fn new() -> Self {
        Self { domain_terms: Vec::new() }
    }

    pub fn add_domain_term(&mut self, term: OntologyTerm) {
        self.domain_terms.push(term);
    }

    pub fn load_domains_json(&mut self, dir: &std::path::Path) -> Result<usize, crate::CoreError> {
        let mut count = 0;
        if !dir.exists() { return Ok(0); }
        for entry in std::fs::read_dir(dir).map_err(|e| crate::CoreError::Internal {
            message: format!("读取领域目录失败: {}", e),
        })? {
            let path = entry.map_err(|e| crate::CoreError::Internal {
                message: format!("读取目录项失败: {}", e),
            })?.path();
            if path.extension().map_or(false, |e| e == "json") {
                let content = std::fs::read_to_string(&path).map_err(|e| crate::CoreError::Internal {
                    message: format!("读取领域文件 {} 失败: {}", path.display(), e),
                })?;
                let domain: serde_json::Value = serde_json::from_str(&content).map_err(|e| crate::CoreError::InvalidJsonLd {
                    message: format!("领域文件 {} JSON 解析失败: {}", path.display(), e),
                })?;
                let ns = domain["namespace"].as_str().unwrap_or("");
                if let Some(terms) = domain["terms"].as_array() {
                    for t in terms {
                        let iri = format!("{}{}", ns, t["id"].as_str().unwrap_or(""));
                        let label = t["label"].as_str().unwrap_or("").to_string();
                        let desc = t["description"].as_str().unwrap_or("").to_string();
                        let tt = match t["type"].as_str().unwrap_or("Class") {
                            "Relation" => OntologyTermType::Relation,
                            "Property" => OntologyTermType::Property,
                            _ => OntologyTermType::Class,
                        };
                        self.domain_terms.push(OntologyTerm { iri, label, description: desc, term_type: tt });
                        count += 1;
                    }
                }
            }
        }
        Ok(count)
    }

    pub fn get_vocabulary(&self, domain: Option<&str>) -> Vec<OntologyTerm> {
        let mut all = Self::built_in_terms().clone();
        all.extend(self.domain_terms.clone());
        match domain {
            Some(d) => all.into_iter().filter(|t| t.iri.contains(d)).collect(),
            None => all,
        }
    }

    pub fn format_vocabulary_for_prompt(&self, terms: &[OntologyTerm]) -> String {
        let mut classes: Vec<&OntologyTerm> = Vec::new();
        let mut properties: Vec<&OntologyTerm> = Vec::new();
        let mut relations: Vec<&OntologyTerm> = Vec::new();
        for t in terms {
            match t.term_type {
                OntologyTermType::Class => classes.push(t),
                OntologyTermType::Property => properties.push(t),
                OntologyTermType::Relation => relations.push(t),
            }
        }
        let mut result = String::new();
        if !classes.is_empty() {
            result.push_str("## 可用实体类型\n");
            for c in &classes {
                result.push_str(&format!(
                    "- IRI: {} | 名称: {} | {}\n",
                    c.iri, c.label, c.description
                ));
            }
        }
        if !properties.is_empty() {
            result.push_str("## 可用属性\n");
            for p in &properties {
                result.push_str(&format!(
                    "- IRI: {} | 名称: {} | {}\n",
                    p.iri, p.label, p.description
                ));
            }
        }
        if !relations.is_empty() {
            result.push_str("## 可用关系\n");
            for r in &relations {
                result.push_str(&format!(
                    "- IRI: {} | 名称: {} | {}\n",
                    r.iri, r.label, r.description
                ));
            }
        }
        result
    }

    fn built_in_terms() -> &'static Vec<OntologyTerm> {
        BUILT_IN_ONTOLOGY.get_or_init(|| {
            let mut terms = vec![
                // ═══════ Core: Agent OS engine (10 classes) ═══════
                OntologyTerm::class(core("Agent"),      "执行者",    "PA/DA/CA/AA"),
                OntologyTerm::class(core("Task"),       "任务",      "用户输入触发的执行单元"),
                OntologyTerm::class(core("Plan"),       "计划",      "PA 生成的步骤序列"),
                OntologyTerm::class(core("Action"),     "动作",      "工具调用记录"),
                OntologyTerm::class(core("File"),       "文件",      "文件实体(路径/SHA256)"),
                OntologyTerm::class(core("Decision"),   "决策",      "CA/AA 审计结论"),
                OntologyTerm::class(core("Session"),    "会话",      "L1 session 摘要"),
                OntologyTerm::class(core("Error"),      "错误",      "执行失败的详情"),
                OntologyTerm::class(core("Metric"),     "指标",      "token/耗时/轮数"),
                OntologyTerm::class(core("Goal"),       "目标",      "预期成果"),

                // ═══════ Engineering: task/project (14 classes) ═══════
                OntologyTerm::class(eng("Requirement"), "需求",      "形式化要求说明"),
                OntologyTerm::class(eng("Deliverable"), "交付物",    "有验收标准的产出"),
                OntologyTerm::class(eng("Issue"),       "问题",      "执行中的障碍/缺陷"),
                OntologyTerm::class(eng("Risk"),        "风险",      "潜在负面结果"),
                OntologyTerm::class(eng("Resource"),    "资源",      "可消耗项(token/配额)"),
                OntologyTerm::class(eng("Milestone"),   "里程碑",    "进度检查点"),
                OntologyTerm::class(eng("Change"),      "变更",      "对已有产物的修改"),
                OntologyTerm::class(eng("Constraint"),  "约束",      "限制执行的条件"),
                OntologyTerm::class(eng("Review"),      "评审",      "对产物的形式化审查"),
                OntologyTerm::class(eng("Test"),        "测试",      "正确性验证"),
                OntologyTerm::class(eng("Pattern"),     "模式",      "可复用经验/教训"),
                OntologyTerm::class(eng("Artifact"),    "产物",      "通用产出(File 父类)"),
                OntologyTerm::class(eng("Project"),     "项目",      "Task 容器"),
                OntologyTerm::class(eng("Role"),        "角色",      "Plan/Do/Check/Act"),

                // ═══════ Code: code understanding (10 classes) ═══════
                OntologyTerm::class(code("Function"),   "函数",      ""),
                OntologyTerm::class(code("Struct"),     "结构体",    ""),
                OntologyTerm::class(code("Enum"),       "枚举",      ""),
                OntologyTerm::class(code("Trait"),      "特质",      ""),
                OntologyTerm::class(code("Class"),      "类",        ""),
                OntologyTerm::class(code("Interface"),  "接口",      ""),
                OntologyTerm::class(code("Impl"),       "实现块",    ""),
                OntologyTerm::class(code("Module"),     "模块",      ""),
                OntologyTerm::class(code("Calls"),      "调用关系",  ""),
                OntologyTerm::class(code("DependsOn"),  "依赖关系",  ""),

                // ═══════ Business: domain compatibility (7 classes) ═══════
                OntologyTerm::class(biz("Person"),       "人物",     ""),
                OntologyTerm::class(biz("Organization"),  "组织",     ""),
                OntologyTerm::class(biz("Product"),       "产品",     ""),
                OntologyTerm::class(biz("Project"),       "业务项目", ""),
                OntologyTerm::class(core("Concept"),      "抽象概念", ""),
                OntologyTerm::class(core("Event"),        "事件",     ""),
                OntologyTerm::class(core("Knowledge"),    "知识片段", ""),

                // ═══════ Relations: 20 core + engineering relations ═══════
                OntologyTerm::relation(core("generatedBy"), "由谁生成",  "Action→Agent"),
                OntologyTerm::relation(core("hasSubTask"),  "包含子任务","Task 分解"),
                OntologyTerm::relation(core("produces"),    "产出",     "→Artifact/File"),
                OntologyTerm::relation(core("refersTo"),    "引用",     "→KnowledgeRef"),
                OntologyTerm::relation(core("followsPlan"), "遵循计划", "Action→Plan"),
                OntologyTerm::relation(core("auditedBy"),   "审计方",   "Decision→Agent"),
                OntologyTerm::relation(core("dependsOn"),   "依赖",     "跨 Task"),
                OntologyTerm::relation(core("assignedTo"),  "分配给",   "Task→Agent"),
                OntologyTerm::relation(eng("addresses"),    "满足需求", "Action→Requirement"),
                OntologyTerm::relation(eng("resolves"),     "解决问题", "Action→Issue"),
                OntologyTerm::relation(eng("blocks"),       "阻塞",     "Issue→Task"),
                OntologyTerm::relation(eng("validates"),    "验证",     "Review→Deliverable"),
                OntologyTerm::relation(eng("constrains"),   "约束",     "Constraint→Task"),
                OntologyTerm::relation(eng("risks"),        "威胁",     "Risk→Task"),
                OntologyTerm::relation(eng("consumes"),     "消耗",     "Action→Resource"),
                OntologyTerm::relation(eng("marks"),        "标记里程碑","Milestone→Task"),
                OntologyTerm::relation(eng("captures"),     "捕获知识", "→Knowledge"),
                OntologyTerm::relation(eng("generalizes"),  "泛化模式", "Pattern→Knowledge"),
                OntologyTerm::relation(eng("prioritizes"),  "确定优先级","Plan→Requirement"),
                OntologyTerm::relation(eng("reports"),      "度量",     "Metric→Goal"),
                // Legacy compatibility
                OntologyTerm::relation(biz("worksFor"),     "就职于", "Person→Org"),
                OntologyTerm::relation(biz("manages"),      "管理",    "Person→Project"),
                OntologyTerm::relation(core("hasSkill"),    "拥有技能", ""),
                OntologyTerm::relation(core("applicableIn"),"适用于",  ""),

                // ═══════ Properties (10) ═══════
                OntologyTerm::property(core("hasStatus"),   "状态"),
                OntologyTerm::property(core("filePath"),    "文件路径"),
                OntologyTerm::property(core("fileHash"),    "文件 SHA256"),
                OntologyTerm::property(core("tokenCost"),   "Token 消耗"),
                OntologyTerm::property(core("duration"),    "耗时(秒)"),
                OntologyTerm::property(core("createdAt"),   "创建时间"),
                OntologyTerm::property(core("completedAt"), "完成时间"),
                OntologyTerm::property(eng("priority"),     "优先级"),
                OntologyTerm::property(eng("severity"),     "严重程度"),
                OntologyTerm::property(core("confidence"),  "置信度"),
            ];
            terms.sort_by_key(|t| t.term_type.clone() as u8);
            terms
        })
    }
}
