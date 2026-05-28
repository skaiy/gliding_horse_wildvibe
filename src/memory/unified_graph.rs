use std::sync::Arc;
use oxigraph::store::Store;
use tracing::info;

/// 统一 Oxigraph 存储 — 系统中唯一的 Oxigraph Store 实例
/// 各领域通过命名图（Named Graph）实现命名空间隔离
pub struct UnifiedGraphStore {
    store: Arc<Store>,
}

impl UnifiedGraphStore {
    /// 创建内存统一存储
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        info!("Initializing Unified Oxigraph Store (memory)");
        Ok(Self {
            store: Arc::new(Store::new()?),
        })
    }

    /// 获取底层 Store 的 Arc 引用，供各子模块共享
    pub fn store(&self) -> Arc<Store> {
        self.store.clone()
    }

    /// 返回内部引用计数（用于诊断）
    pub fn ref_count(&self) -> usize {
        Arc::strong_count(&self.store)
    }
}