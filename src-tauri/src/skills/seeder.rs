//! Built-in demo skills that ship with nebula.
//!
//! These skills are seeded into the skill store on first bootstrap
//! so the Skill Browser is never empty. Each skill is intentionally
//! simple — they serve as examples users can inspect, modify, or
//! use as templates for their own skills.

use tracing::info;

use crate::skills::types::{CreateSkillRequest, Skill};

/// Seed three demo skills into the skill engine.
///
/// Idempotent: skips any skill whose name already exists in the store.
pub fn seed_demo_skills(engine: &crate::skills::engine::SkillEngine) -> anyhow::Result<Vec<Skill>> {
    let demo_skills: Vec<CreateSkillRequest> = vec![
        // Skill 1: Hello World (Python)
        CreateSkillRequest {
            name: "hello-world".into(),
            description: "Prints a greeting. The simplest possible skill — use it to verify the skill engine works.".into(),
            code: r#"print("Hello from nebula!")
import platform
print(f"Python {platform.python_version()} on {platform.system()}")
"#.into(),
            language: "python".into(),
            tags: vec!["demo".into(), "beginner".into()],
            source_memory_id: None,
            ..Default::default()
        },
        // Skill 2: File Summary (Python)
        CreateSkillRequest {
            name: "file-summary".into(),
            description: "Reads a file and prints line count, word count, and byte size. Useful for quick file inspection.".into(),
            code: r#"import os
import sys

# Read FILENAME from params or use first argument
filename = os.environ.get("SKILL_FILE", "")
if not filename and len(sys.argv) > 1:
    filename = sys.argv[1]

if not filename:
    print("Usage: set SKILL_FILE=/path/to/file or pass as argument")
    exit(1)

if not os.path.exists(filename):
    print(f"File not found: {filename}")
    exit(1)

size = os.path.getsize(filename)
with open(filename, "r", encoding="utf-8", errors="replace") as f:
    lines = f.readlines()

words = sum(len(line.split()) for line in lines)
print(f"File: {os.path.basename(filename)}")
print(f"Lines: {len(lines)}")
print(f"Words: {words}")
print(f"Size:  {size} bytes")
"#.into(),
            language: "python".into(),
            tags: vec!["demo".into(), "file".into(), "utility".into()],
            source_memory_id: None,
            ..Default::default()
        },
        // Skill 3: Code Review Prompt (LLM)
        CreateSkillRequest {
            name: "code-review".into(),
            description: "Generates a structured code-review prompt for the given code snippet. Paste code into the skill input and send to any LLM agent.".into(),
            code: r#"You are a senior code reviewer. Review the following code and provide:

1. **Bugs & Edge Cases**: What could go wrong?
2. **Style & Readability**: Naming, structure, comments
3. **Performance**: Bottlenecks or unnecessary work
4. **Suggestions**: Concrete improvements with before/after examples

Be concise. Flag severity: [critical] [warning] [nit].

--- CODE TO REVIEW ---
{{INPUT}}
"#.into(),
            language: "llm".into(),
            tags: vec!["demo".into(), "code".into(), "review".into()],
            source_memory_id: None,
            ..Default::default()
        },
        // T-E-S-38 Skill 4: canvas-creator (LLM)
        // 约束 LLM 输出 <<<HTML>>>...<<<END>>> 包裹的完整 HTML(含 <canvas> + 内联 JS)。
        CreateSkillRequest {
            name: "canvas-creator".into(),
            description: "Generate a standalone HTML5 Canvas visualization from a natural-language description. Outputs a complete HTML document wrapped in <<<HTML>>>...<<<END>>> markers.".into(),
            code: r#"You are a visualization engineer. Given a description, produce a complete standalone HTML5 document that renders the requested visualization on a <canvas> element with inline JavaScript.

# Output format (STRICT)
Wrap the entire HTML document between the markers below — nothing else may appear in your reply:

<<<HTML>>>
<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8" />
  <style>
    /* center the canvas, dark theme, no margin */
    body { margin: 0; background: #1E293B; display: flex; justify-content: center; align-items: center; min-height: 100vh; }
  </style>
</head>
<body>
  <canvas id="viz" width="800" height="600"></canvas>
  <script>
    // drawing code — vanilla JS only, no external libraries
  </script>
</body>
</html>
<<<END>>>

# Rules
- Output ONLY the wrapped HTML block. No prose, no explanations, no markdown fences.
- Use vanilla JavaScript — no external scripts, no import statements, no fetch calls.
- The canvas must be self-contained and render immediately on page load.
- Honor the user's description: chart type, labels, colors, data points.

--- USER DESCRIPTION ---
{{INPUT}}
"#.into(),
            language: "llm".into(),
            tags: vec!["visualization".into(), "canvas".into(), "html".into()],
            source_memory_id: None,
            capabilities: crate::skills::sandbox::CapabilitySet::llm_only(),
            ..Default::default()
        },
        // T-E-S-38 Skill 5: mermaid-creator (LLM)
        // 约束 LLM 输出 ```mermaid ... ``` fenced code block。
        CreateSkillRequest {
            name: "mermaid-creator".into(),
            description: "Generate Mermaid diagram code (flowchart / sequence / gantt / state / class) from a natural-language description. Outputs a single fenced ```mermaid code block.".into(),
            code: r#"You are a diagram designer. Given a description, produce a Mermaid diagram that visualizes the requested flow, sequence, gantt, state, or class diagram.

# Output format (STRICT)
Output ONLY one fenced code block in this exact form — no prose before or after:

```mermaid
flowchart TD
  A[Start] --> B{Decision}
  B -->|Yes| C[Do thing]
  B -->|No| D[Skip]
```

# Rules
- Pick the most appropriate diagram type from: flowchart / sequenceDiagram / gantt / stateDiagram-v2 / classDiagram.
- Output a SINGLE ```mermaid fenced block. Do not add explanations.
- Use valid Mermaid syntax only (no unsupported extensions).
- Honor the user's description: nodes, edges, labels, branches.

--- USER DESCRIPTION ---
{{INPUT}}
"#.into(),
            language: "llm".into(),
            tags: vec!["visualization".into(), "mermaid".into(), "diagram".into()],
            source_memory_id: None,
            capabilities: crate::skills::sandbox::CapabilitySet::llm_only(),
            ..Default::default()
        },
        // T-E-S-38 Skill 6: mindmap-creator (LLM)
        // 约束 LLM 输出 Mermaid mindmap 语法。
        CreateSkillRequest {
            name: "mindmap-creator".into(),
            description: "Generate a Mermaid mindmap diagram from a natural-language description. Outputs a single fenced ```mermaid code block using the mindmap syntax.".into(),
            code: r#"You are a mind-map designer. Given a topic or description, produce a Mermaid mindmap that radiates from a central node into related sub-topics.

# Output format (STRICT)
Output ONLY one fenced code block in this exact form — no prose before or after:

```mermaid
mindmap
  root((Topic))
    Branch A
      Subtopic A1
      Subtopic A2
    Branch B
      Subtopic B1
```

# Rules
- Use the Mermaid `mindmap` syntax (top-level keyword must be `mindmap`).
- Output a SINGLE ```mermaid fenced block. Do not add explanations.
- 3-5 main branches radiating from the root, each with 2-4 subtopics.
- Honor the user's description: topic, depth, key areas.

--- USER DESCRIPTION ---
{{INPUT}}
"#.into(),
            language: "llm".into(),
            tags: vec!["visualization".into(), "mindmap".into(), "mermaid".into()],
            source_memory_id: None,
            capabilities: crate::skills::sandbox::CapabilitySet::llm_only(),
            ..Default::default()
        },
    ];

    let mut created = Vec::new();

    for req in demo_skills {
        // Idempotent: skip if already seeded
        let existing = engine.list_skills(crate::skills::types::ListSkillsRequest {
            language: None,
            tag: None,
            limit: 100,
            ..Default::default()
        })?;

        if existing.iter().any(|s| s.name == req.name) {
            info!(
                target: "nebula.skills.seed",
                name = %req.name,
                "demo skill already exists, skipping"
            );
            continue;
        }

        match engine.create_skill(req.clone()) {
            Ok(skill) => {
                info!(
                    target: "nebula.skills.seed",
                    name = %skill.name,
                    id = %skill.id,
                    "seeded demo skill"
                );
                created.push(skill);
            }
            Err(e) => {
                // Don't fail bootstrap for a demo skill
                tracing::warn!(
                    target: "nebula.skills.seed",
                    name = %req.name,
                    error = ?e,
                    "failed to seed demo skill"
                );
            }
        }
    }

    Ok(created)
}

// ---------------------------------------------------------------------------
// T-E-S-38 单测:三个可视化 creator skill 的 seeding + capability 匹配
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::gateway::LlmGateway;
    use crate::llm::ollama::OllamaClient;
    use crate::memory::sqlite_store::SqliteStore;
    use crate::skills::capability::Capability;
    use crate::skills::engine::SkillEngine;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    /// 构造一个临时 SQLite + SkillEngine,运行 bundled migrations。
    fn temp_engine() -> (PathBuf, SkillEngine) {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "nebula_seeder_viz_test_{}.db",
            uuid::Uuid::new_v4()
        ));
        let sqlite = Arc::new(SqliteStore::open(&p).unwrap());
        {
            let rc = sqlite.raw_connection();
            let g = rc.lock();
            crate::memory::migration::run_migrations(
                &g,
                crate::memory::migration::bundled_migrations_dir(),
            )
            .unwrap();
        }
        let client = Arc::new(OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            std::time::Duration::from_secs(2),
        ));
        let llm = Arc::new(LlmGateway::new(
            client, "m", "ollama", None, None, None, None, None,
        ));
        let engine = SkillEngine::new(sqlite, llm);
        (p, engine)
    }

    fn cleanup(p: &Path) {
        let _ = std::fs::remove_file(p);
        let _ = std::fs::remove_file(p.with_extension("db-wal"));
        let _ = std::fs::remove_file(p.with_extension("db-shm"));
    }

    /// 列出全部 skills(limit 提到 200 防止截断)。
    fn list_all(engine: &SkillEngine) -> Vec<Skill> {
        engine
            .list_skills(crate::skills::types::ListSkillsRequest {
                language: None,
                tag: None,
                limit: 200,
                ..Default::default()
            })
            .unwrap()
    }

    /// T-E-S-38 验收:seed_demo_skills 后 list_skills 包含三个 creator。
    #[test]
    fn seed_demo_skills_includes_three_viz_creators() {
        let (p, engine) = temp_engine();
        seed_demo_skills(&engine).unwrap();
        let skills = list_all(&engine);
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"canvas-creator"), "canvas-creator missing");
        assert!(
            names.contains(&"mermaid-creator"),
            "mermaid-creator missing"
        );
        assert!(
            names.contains(&"mindmap-creator"),
            "mindmap-creator missing"
        );
        cleanup(&p);
    }

    /// T-E-S-38 验收:seed 幂等(连续两次 creator 数量不翻倍)。
    #[test]
    fn seed_demo_skills_is_idempotent_for_viz_creators() {
        let (p, engine) = temp_engine();
        seed_demo_skills(&engine).unwrap();
        let after_first = list_all(&engine)
            .into_iter()
            .filter(|s| {
                s.name == "canvas-creator"
                    || s.name == "mermaid-creator"
                    || s.name == "mindmap-creator"
            })
            .count();
        // 第二次 seed 应全部跳过。
        seed_demo_skills(&engine).unwrap();
        let after_second = list_all(&engine)
            .into_iter()
            .filter(|s| {
                s.name == "canvas-creator"
                    || s.name == "mermaid-creator"
                    || s.name == "mindmap-creator"
            })
            .count();
        assert_eq!(
            after_first, after_second,
            "idempotent seed must not duplicate creators: first={after_first} second={after_second}"
        );
        assert_eq!(after_first, 3, "expected exactly 3 creators after seeding");
        cleanup(&p);
    }

    /// T-E-S-38 验收:三个 creator language == "llm"。
    #[test]
    fn three_viz_creators_use_llm_language() {
        let (p, engine) = temp_engine();
        seed_demo_skills(&engine).unwrap();
        let skills = list_all(&engine);
        for name in ["canvas-creator", "mermaid-creator", "mindmap-creator"] {
            let s = skills
                .iter()
                .find(|s| s.name == name)
                .unwrap_or_else(|| panic!("{name} should exist after seed"));
            assert_eq!(
                s.language, "llm",
                "{name} must be language=llm (got {:?})",
                s.language
            );
        }
        cleanup(&p);
    }

    /// T-E-S-38 验收:三个 creator capabilities 非空(应至少含 LlmCall)。
    #[test]
    fn three_viz_creators_have_nonempty_capabilities() {
        let (p, engine) = temp_engine();
        seed_demo_skills(&engine).unwrap();
        let skills = list_all(&engine);
        for name in ["canvas-creator", "mermaid-creator", "mindmap-creator"] {
            let s = skills
                .iter()
                .find(|s| s.name == name)
                .unwrap_or_else(|| panic!("{name} should exist after seed"));
            assert!(
                !s.capabilities.is_empty(),
                "{name} capabilities must be non-empty"
            );
            assert!(
                s.capabilities
                    .has(crate::skills::sandbox::Capability::LlmCall),
                "{name} must have LlmCall capability"
            );
        }
        cleanup(&p);
    }

    /// T-E-S-38 验收:match_by_intent("canvas 流程图") 命中 viz:canvas。
    ///
    /// 注意:intent 关键词匹配是按空白分割的小写子串匹配,因此 "canvas"
    /// 必须出现在 capability 的 name / description / skills 之一。
    #[test]
    fn match_by_intent_canvas_hits_viz_canvas() {
        let (p, engine) = temp_engine();
        // 注册 viz:canvas capability(name 含 "canvas" 关键词)。
        engine.register_capability(Capability {
            id: "viz:canvas".to_string(),
            name: "Canvas Visualization".to_string(),
            description: "Generate HTML5 canvas visualizations from natural language".to_string(),
            skills: vec!["canvas-creator".to_string()],
        });
        let hits = engine.match_capabilities_by_intent("canvas 流程图");
        assert!(
            hits.iter().any(|c| c.id == "viz:canvas"),
            "intent 'canvas 流程图' should hit viz:canvas, got {:?}",
            hits.iter().map(|c| &c.id).collect::<Vec<_>>()
        );
        cleanup(&p);
    }

    /// T-E-S-38 验收:match_by_input({ skill: "mermaid-creator" }) 命中 viz:mermaid。
    #[test]
    fn match_by_input_skill_mermaid_hits_viz_mermaid() {
        let (p, engine) = temp_engine();
        engine.register_capability(Capability {
            id: "viz:mermaid".to_string(),
            name: "Mermaid Diagram".to_string(),
            description: "Generate Mermaid flowchart / sequence / gantt diagrams".to_string(),
            skills: vec!["mermaid-creator".to_string()],
        });
        let input = serde_json::json!({ "skill": "mermaid-creator" });
        let hits = engine.match_capabilities_by_input(&input);
        assert!(
            hits.iter().any(|c| c.id == "viz:mermaid"),
            "input {{ skill: 'mermaid-creator' }} should hit viz:mermaid, got {:?}",
            hits.iter().map(|c| &c.id).collect::<Vec<_>>()
        );
        cleanup(&p);
    }
}
