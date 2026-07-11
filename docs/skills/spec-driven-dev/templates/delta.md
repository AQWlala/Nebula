---
change: {{CHANGE_NAME}}
domain: {{DOMAIN}}
status: draft
created: {{DATE}}
---

# Delta for {{DOMAIN}}

> **Change**: {{CHANGE_NAME}}
> **Domain**: {{DOMAIN}}
> **Created**: {{DATE}}
> **编写指南**: [DELTA_SPEC_GUIDE.md](../../DELTA_SPEC_GUIDE.md)

<!--
  Delta spec 只描述"什么变了",不重写整个文档。
  使用三种语义标记: ADDED / MODIFIED / REMOVED
  详细规范见 DELTA_SPEC_GUIDE.md。

  编写检查清单:
  - [ ] 每个 ADDED requirement 是全新的(不在当前 spec 中)
  - [ ] 每个 MODIFIED requirement 在当前 spec 中存在
  - [ ] 每个 REMOVED requirement 在当前 spec 中存在
  - [ ] 每个 requirement 至少有一个 scenario
  - [ ] 每个 scenario 有 WHEN + THEN
  - [ ] Scenario 可测试(能转化为 assert)
  - [ ] MODIFIED requirement 写出了完整新版本(不是 diff)
  - [ ] 使用 RFC 2119 关键词(MUST/SHALL/SHOULD/MAY)
-->

## ADDED Requirements

<!-- 新增的 requirement。每个 requirement 至少一个 scenario。
     格式:
     ### Requirement: <中文名词短语>
     The system SHALL <行为描述>.

     - <细节点>

     #### Scenario: <场景名称>
     - **WHEN** <触发条件>
     - **THEN** <预期行为>
     - **AND** <附加约束>
     - **AND NOT** <不应发生的行为>(可选)
-->

### Requirement: [新增 requirement 名称]
The system SHALL [行为描述].

- [细节点 1]
- [细节点 2]

#### Scenario: [场景名称]
- **WHEN** [触发条件]
- **THEN** [预期行为]
- **AND** [附加约束]

## MODIFIED Requirements

<!-- 修改现有 requirement。必须写出完整的新版本(不是 diff)。
     被修改的 requirement 必须在当前 spec.md 中存在。
     格式:
     ### Requirement: <现有 requirement 名称>
     <新的完整行为描述> (Previously: <之前的行为摘要>)

     - <变更说明>

     #### Scenario: <场景名称>
     - **WHEN** <触发条件>
     - **THEN** <新的预期行为>
-->

### Requirement: [现有 requirement 名称]
[新的完整行为描述] (Previously: [之前的行为摘要])

- [变更说明: 之前是 X,现在是 Y]

#### Scenario: [场景名称]
- **WHEN** [触发条件]
- **THEN** [新的预期行为]

## REMOVED Requirements

<!-- 删除现有 requirement。必须提供删除原因和迁移说明。
     被删除的 requirement 必须在当前 spec.md 中存在。
     格式:
     ### Requirement: <现有 requirement 名称>
     [删除原因]

     **迁移说明**: [替代方案]
     **影响**: [受影响的文件/模块]
-->

### Requirement: [现有 requirement 名称]
[删除原因和影响说明]

**迁移说明**: [替代方案]
**影响**:
- [受影响的文件/模块]

<!--
  若无某类变更,写 (none):
  ## ADDED Requirements
  (none)
-->

---

*本文件是 change `{{CHANGE_NAME}}` 的 Delta spec。归档时会合并到
`openspec/specs/{{DOMAIN}}/spec.md`。*
