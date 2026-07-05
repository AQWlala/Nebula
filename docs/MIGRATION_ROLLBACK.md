# 数据库迁移回滚策略

> **关联**: ADR-004 §6 回滚策略, M7b #96 数据库迁移验证
> **最后更新**: 2026-07-05

---

## 1. 设计哲学:前向幂等 + 备份恢复

nebula 项目采用**前向幂等(forward-idempotent)**模式,而非**可逆(reversible)**模式:

- **前向幂等**:迁移文件用 `IF NOT EXISTS` / `is_idempotent_error` 兜底,重复应用不报错
- **备份恢复**:迁移前自动创建 `.bak` 备份,失败时手动恢复

不提供 down migration runner 的原因:
1. SQLite 的 `ALTER TABLE DROP COLUMN` 在旧版本不支持(SQLite 3.35.0+)
2. 数据丢失不可逆(如 `DROP TABLE` 后数据无法恢复)
3. 备份恢复比 down migration 更安全(完整状态回滚)

---

## 2. 自动备份策略(M7b #96 实现)

### 2.1 备份触发

`run_bundled_migrations()` 在应用 pending migrations 之前,检测到数据库有文件路径(非 `:memory:`)时,自动用 `VACUUM INTO` 创建一致性快照。

### 2.2 备份位置

```
<db_dir>/<db_name>.migrate_v<from>_to_v<to>.bak
```

示例:`nebula.db.migrate_v35_to_v36.bak`

### 2.3 备份失败处理

备份失败仅记 `warn` 日志,不阻塞迁移流程(避免因磁盘空间不足等次要问题阻止启动)。

### 2.4 跳过条件

- `:memory:` 数据库(无文件路径)
- 无 pending migrations(已是最新的库)
- 获取数据库路径失败

---

## 3. 手动回滚步骤

### 3.1 标准回滚(推荐)

当迁移失败或新版本有问题时,用备份恢复:

```powershell
# 1. 关闭 nebula 应用
# 2. 定位数据库和备份文件
$dbDir = "$env:LOCALAPPDATA\com.nebula.desktop"
$db = "$dbDir\nebula.db"
$bak = Get-ChildItem "$dbDir\*.migrate_v*.bak" | Sort-Object LastWriteTime -Descending | Select-Object -First 1

# 3. 用备份覆盖当前数据库
Copy-Item $bak.FullName $db -Force

# 4. 删除 WAL 和 SHM 文件(确保干净启动)
Remove-Item "$db-wal", "$db-shm" -ErrorAction SilentlyContinue

# 5. 重启应用
```

### 3.2 部分回滚(降级 user_version)

若仅需跳过某个迁移(如 036 有问题但 035 已应用成功):

```sql
-- 在 SQLite CLI 中执行
PRAGMA user_version = 35;  -- 回退到 035 应用后的状态
-- 然后手动执行 036 的回滚 SQL(见下方)
```

---

## 4. 各迁移的回滚 SQL

> **警告**:回滚会导致数据丢失。仅在备份恢复不可行时使用。
> 所有 `ALTER TABLE DROP COLUMN` 需要 SQLite 3.35.0+。

| 版本 | 迁移 | 回滚 SQL | 数据影响 |
|------|------|---------|---------|
| 002 | reflections | `DROP TABLE IF EXISTS reflections;` | 删除反思记录 |
| 003 | skills | `DROP TABLE IF EXISTS skills;` | 删除技能定义 |
| ... | ... | ... | ... |
| 027 | cost_source | `DROP INDEX IF EXISTS idx_cost_records_source; DROP TABLE IF EXISTS cost_records;` | 删除成本记录 |
| 030 | ingest_cost | `ALTER TABLE memories DROP COLUMN ingest_cost;` | 丢失 ingest_cost 字段 |
| 034 | arena | `DROP TABLE IF EXISTS model_elo_scores; DROP TABLE IF EXISTS arena_matches;` | 删除 arena 对战记录 |
| 035 | domain_column | `DROP INDEX IF EXISTS idx_memories_domain; ALTER TABLE memories DROP COLUMN domain;` | 丢失 domain 字段(回退到无域隔离) |
| 036 | cost_work_type | `DROP INDEX IF EXISTS idx_cost_records_work_type; ALTER TABLE cost_records DROP COLUMN work_type;` | 丢失 work_type 字段(回退到无 WorkType 分域统计) |

---

## 5. 回滚后的一致性检查

回滚后,应用代码可能引用已删除的列/表。需要:

1. **降级应用版本**:回滚到迁移应用前的 nebula 版本
2. **清理 WAL/SHM**:删除 `-wal` 和 `-shm` 文件
3. **验证 schema**:`PRAGMA integrity_check;`
4. **检查 user_version**:`PRAGMA user_version;` 确认与代码期望一致

---

## 6. 紧急情况:无备份时的恢复

若自动备份失败且数据库损坏:

1. **尝试 `.recover` 命令**(SQLite CLI):
   ```bash
   sqlite3 corrupted.db ".recover" > recovered.sql
   sqlite3 new.db < recovered.sql
   ```

2. **使用 BackupScheduler 的定期备份**:
   - 位置:`%LOCALAPPDATA%\nebula\backups\`
   - 由 `BackupScheduler` 后台调度(与迁移无关,但可作灾备)

3. **最后手段**:从 `nebula.db.bak`(CipherMigrator 创建,若启用加密)+ 手动重建
