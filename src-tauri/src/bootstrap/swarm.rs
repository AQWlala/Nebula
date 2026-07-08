use std::sync::Arc;

use tracing::info;

use crate::app_config::AppConfig;
use crate::app_state::AppState;
use crate::llm::gateway::LlmGateway;
use crate::memory::embedder::Embedder;
use crate::memory::reflect::{ReflectConfig, ReflectionEngine};
use crate::memory::sponge::SpongeEngine;
use crate::memory::sqlite_store::SqliteStore;
use crate::memory::vector_store::VectorStore;
use crate::skills::audit::SkillAuditLogger;
use crate::skills::engine::SkillEngine;
use crate::skills::extractor::SkillExtractor;
use crate::skills::importer::SkillImporter;
use crate::skills::store::SkillStore;
use crate::swarm::composer::SkillComposer;
use crate::swarm::orchestrator::SwarmOrchestrator;
use crate::tools::ToolRegistry;

impl AppState {
    pub(crate) fn bootstrap_swarm_and_reflection(
        config: &AppConfig,
        sqlite: &Arc<SqliteStore>,
        lance: &Arc<dyn VectorStore>,
        embedder: &Arc<Embedder>,
        llm: &Arc<LlmGateway>,
        sponge: &Arc<SpongeEngine>,
        tool_registry: &Arc<ToolRegistry>,
    ) -> (
        Arc<SwarmOrchestrator>,
        Arc<ReflectionEngine>,
        Arc<crate::swarm::DeadlockDetector>,
    ) {
        let swarm = Arc::new(SwarmOrchestrator::new(
            llm.clone(),
            sponge.clone(),
            lance.clone(),
            embedder.clone(),
            sqlite.clone(),
            tool_registry.clone(),
        ));
        let cfg = ReflectConfig {
            window_days: config.reflect_window_days,
            min_importance: config.reflect_min_importance,
            worker_interval_secs: config.reflect_interval_secs,
            ..ReflectConfig::default()
        };
        let reflection = Arc::new(ReflectionEngine::new(
            sqlite.clone(),
            Some(llm.clone()),
            cfg,
        ));
        let mut deadlock_detector = crate::swarm::DeadlockDetector::with_bus(swarm.bus());
        deadlock_detector.start();
        let deadlock_detector = Arc::new(deadlock_detector);
        (swarm, reflection, deadlock_detector)
    }

    pub(crate) fn bootstrap_skills(
        config: &AppConfig,
        sqlite: &Arc<SqliteStore>,
        llm: &Arc<LlmGateway>,
        exec_approval: &Arc<crate::skills::exec_approval::ExecApprovalTracker>,
    ) -> (
        Arc<SkillEngine>,
        Arc<SkillExtractor>,
        Arc<SkillComposer>,
        Arc<crate::skills::SkillMarketplace>,
        Arc<SkillAuditLogger>,
    ) {
        let ss = Arc::new(
            SkillStore::new(sqlite.as_ref().clone()).expect("SkillStore::new must succeed"),
        );
        let audit = Arc::new(SkillAuditLogger::new(sqlite.raw_connection()));
        let skills = Arc::new(
            SkillEngine::from_store((*ss).clone(), llm.clone())
                .with_audit(audit.clone())
                .with_exec_approval(exec_approval.clone()),
        );
        info!(target: "nebula", "exec approval tracker wired into SkillEngine");
        let adir = config
            .db_path
            .rsplit_once(std::path::MAIN_SEPARATOR)
            .map(|(d, _)| d)
            .unwrap_or(".")
            .to_string()
            + "/skills_archive";
        let extr = Arc::new(SkillExtractor::new(llm.clone(), ss.clone(), adir));
        let comp = Arc::new(SkillComposer::new(ss.clone(), Some(llm.clone())));
        let imp = Arc::new(SkillImporter::new((*ss).clone()));
        let mp = Arc::new(crate::skills::SkillMarketplace::new(ss, imp));
        let _ = mp.refresh();
        crate::skills::seed_demo_skills(&skills).unwrap_or_else(|e| {
            tracing::warn!(target: "nebula", error = ?e, "failed to seed demo skills");
            Vec::new()
        });
        skills.register_capability(crate::skills::capability::Capability {
            id: "viz:canvas".to_string(),
            name: "Canvas Visualization".to_string(),
            description: "Generate HTML5 canvas visualizations from natural language".to_string(),
            skills: vec!["canvas-creator".to_string()],
        });
        skills.register_capability(crate::skills::capability::Capability {
            id: "viz:mermaid".to_string(),
            name: "Mermaid Diagram".to_string(),
            description: "Generate Mermaid flowchart / sequence / gantt / state / class diagrams"
                .to_string(),
            skills: vec!["mermaid-creator".to_string()],
        });
        skills.register_capability(crate::skills::capability::Capability {
            id: "viz:mindmap".to_string(),
            name: "Mermaid Mindmap".to_string(),
            description: "Generate Mermaid mindmap diagrams from a topic".to_string(),
            skills: vec!["mindmap-creator".to_string()],
        });
        info!(
            target: "nebula",
            count = skills.list_capabilities().len(),
            "viz creator capabilities registered (T-E-S-38)"
        );
        (skills, extr, comp, mp, audit)
    }
}
