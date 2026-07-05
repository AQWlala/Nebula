/**
 * v0.5: Work 模式
 *
 * 三栏看板：Todo / Doing / Done
 * 顶部：新建任务 + 会议纪要生成器
 * 卡片：标题 / 描述 / 优先级 / 截止 / 计时器
 *
 * AI 优先级推荐：基于标题关键词 + 截止时间。
 * 时间追踪：单任务计时器，跨页面仍运行（后端保留状态）。
 */
import { useEffect, useMemo, useState } from 'preact/hooks';
import { nebulaAPI, type WorkTask, type WorkTaskStatus } from '../lib/tauri';
import { t } from '../i18n';

const COLUMNS: { id: WorkTaskStatus; labelKey: string; accent: string }[] = [
  { id: 'todo',  labelKey: 'workMode.todo', accent: '#7a8a9a' },
  { id: 'doing', labelKey: 'workMode.doing', accent: '#ffb86b' },
  { id: 'done',  labelKey: 'workMode.done', accent: '#39d98a' },
];

export function WorkMode() {
  const [tasks, setTasks] = useState<WorkTask[]>([]);
  const [activeTimer, setActiveTimer] = useState<string | null>(null);
  const [tickStart, setTickStart] = useState<number>(0);
  const [showMeeting, setShowMeeting] = useState(false);
  const [transcript, setTranscript] = useState('');
  const [meetingOut, setMeetingOut] = useState<{ decisions: string[]; actions: string[] } | null>(null);
  const [newTitle, setNewTitle] = useState('');
  const [newDesc, setNewDesc] = useState('');
  const [newPriority, setNewPriority] = useState(0);
  const [newDue, setNewDue] = useState<string>('');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    refresh();
    nebulaAPI.workActiveTimer().then(setActiveTimer).catch(() => undefined);
  }, []);

  const refresh = async () => {
    try {
      const all = await nebulaAPI.workListTasks(undefined, 200);
      setTasks(all);
    } catch (e) {
      setError(String(e));
    }
  };

  const grouped = useMemo(() => {
    const m: Record<WorkTaskStatus, WorkTask[]> = { todo: [], doing: [], done: [] };
    for (const t of tasks) m[t.status].push(t);
    return m;
  }, [tasks]);

  // 计时器显示（前端只负责呈现）
  useEffect(() => {
    if (!activeTimer) return;
    setTickStart(Date.now());
    const id = setInterval(() => setTickStart(Date.now()), 1000);
    return () => clearInterval(id);
  }, [activeTimer]);

  const onCreate = async () => {
    if (!newTitle.trim()) return;
    try {
      const dueAt = newDue ? Math.floor(new Date(newDue).getTime() / 1000) : null;
      // AI 推荐优先级
      const aiPri = await nebulaAPI.workRecommendPriority(newTitle, dueAt);
      const finalPri = Math.max(newPriority, aiPri);
      await nebulaAPI.workCreateTask({
        title: newTitle.trim(),
        description: newDesc.trim(),
        priority: finalPri,
        due_at: dueAt,
      });
      setNewTitle('');
      setNewDesc('');
      setNewPriority(0);
      setNewDue('');
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const onMove = async (task: WorkTask, status: WorkTaskStatus) => {
    if (task.status === status) return;
    try {
      await nebulaAPI.workSetStatus(task.id, status);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const onStartTimer = async (task: WorkTask) => {
    try {
      if (activeTimer && activeTimer !== task.id) {
        // 把上一个任务的累加器刷一次
        const prev = tasks.find((t) => t.id === activeTimer);
        if (prev) {
          // 我们不知道上一个 start 的精确时刻，所以这里保守地使用
          // 简单的 "stop → start" 调用链。
          await nebulaAPI.workStopTimer();
        }
      }
      await nebulaAPI.workStartTimer(task.id);
      setActiveTimer(task.id);
      // 移动到 doing
      if (task.status !== 'doing') {
        await nebulaAPI.workSetStatus(task.id, 'doing');
      }
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const onStopTimer = async (task: WorkTask) => {
    try {
      const elapsed = Date.now() - tickStart;
      if (elapsed > 0) {
        await nebulaAPI.workAddTime(task.id, elapsed);
      }
      await nebulaAPI.workStopTimer();
      setActiveTimer(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const onDelete = async (task: WorkTask) => {
    if (!confirm(t('workMode.confirmDelete', { title: task.title }))) return;
    try {
      await nebulaAPI.workDeleteTask(task.id);
      if (activeTimer === task.id) setActiveTimer(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const onSummarise = async () => {
    try {
      const out = await nebulaAPI.workSummariseMeeting(transcript);
      setMeetingOut(out);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div class="work-mode">
      <header class="work-toolbar">
        <input
          class="work-new-title"
          placeholder={t('workMode.newTaskPlaceholder')}
          value={newTitle}
          onInput={(e) => setNewTitle((e.target as HTMLInputElement).value)}
          onKeyDown={(e) => { if (e.key === 'Enter') onCreate(); }}
        />
        <input
          class="work-new-desc"
          placeholder={t('workMode.descPlaceholder')}
          value={newDesc}
          onInput={(e) => setNewDesc((e.target as HTMLInputElement).value)}
        />
        <select
          class="work-new-priority"
          value={newPriority}
          onChange={(e) => setNewPriority(parseInt((e.target as HTMLSelectElement).value, 10))}
          title={t('workMode.priorityTitle')}
        >
          <option value={0}>P0</option>
          <option value={1}>P1</option>
          <option value={2}>P2</option>
          <option value={3}>P3</option>
        </select>
        <input
          class="work-new-due"
          type="datetime-local"
          value={newDue}
          onInput={(e) => setNewDue((e.target as HTMLInputElement).value)}
          title={t('workMode.dueTitle')}
        />
        <button class="primary" onClick={onCreate}>{t('workMode.add')}</button>
        <div class="spacer" />
        <button class="ghost" onClick={() => setShowMeeting((v) => !v)}>
          {t('workMode.toggleMeeting', { state: showMeeting ? t('workMode.collapse') : t('workMode.expand') })}
        </button>
      </header>

      {showMeeting && (
        <section class="work-meeting">
          <textarea
            placeholder={t('workMode.meetingPlaceholder')}
            value={transcript}
            onInput={(e) => setTranscript((e.target as HTMLTextAreaElement).value)}
            rows={6}
          />
          <div class="row">
            <button onClick={onSummarise} class="primary">{t('workMode.generateSummary')}</button>
            {meetingOut && (
              <>
                <span class="meeting-stat">{t('workMode.decisionCount', { count: meetingOut.decisions.length })}</span>
                <span class="meeting-stat">{t('workMode.actionCount', { count: meetingOut.actions.length })}</span>
              </>
            )}
          </div>
          {meetingOut && (
            <div class="meeting-result">
              <div>
                <h4>{t('workMode.decisions')}</h4>
                <ol>
                  {meetingOut.decisions.map((d) => (<li key={d}>{d}</li>))}
                </ol>
              </div>
              <div>
                <h4>{t('workMode.actionItems')}</h4>
                <ul>
                  {meetingOut.actions.map((a) => (<li key={a}>{a}</li>))}
                </ul>
              </div>
            </div>
          )}
        </section>
      )}

      {error && <p class="error">{error}</p>}

      <div class="work-kanban">
        {COLUMNS.map((col) => (
          <section
            key={col.id}
            class={`kanban-col kanban-${col.id}`}
            onDragOver={(e) => e.preventDefault()}
            onDrop={(e) => {
              e.preventDefault();
              const id = e.dataTransfer?.getData('text/plain');
              const task = tasks.find((x) => x.id === id);
              if (task) onMove(task, col.id);
            }}
          >
            <header class="col-header" style={{ borderTop: `2px solid ${col.accent}` }}>
              <span>{t(col.labelKey as any)}</span>
              <span class="col-count">{grouped[col.id].length}</span>
            </header>
            <div class="col-body">
              {grouped[col.id].length === 0 ? (
                <p class="empty">{t('workMode.emptyColumn')}</p>
              ) : (
                grouped[col.id].map((task) => (
                  <article
                    key={task.id}
                    class={`task-card priority-${task.priority} ${activeTimer === task.id ? 'is-timed' : ''}`}
                    draggable
                    onDragStart={(e) => e.dataTransfer?.setData('text/plain', task.id)}
                  >
                    <div class="task-head">
                      <span class={`prio prio-${task.priority}`}>P{task.priority}</span>
                      <h4 class="task-title">{task.title}</h4>
                      <button class="task-del" onClick={() => onDelete(task)}>×</button>
                    </div>
                    {task.description && <p class="task-desc">{task.description}</p>}
                    <div class="task-foot">
                      {task.due_at && (
                        <span class="task-due">📅 {formatTime(task.due_at)}</span>
                      )}
                      <span class="task-time">⏱ {formatDuration(task.time_spent_ms)}</span>
                      <div class="spacer" />
                      {activeTimer === task.id ? (
                        <button class="timer-btn running" onClick={() => onStopTimer(task)}>
                          {t('workMode.stopTimer', { duration: formatDuration(Date.now() - tickStart) })}
                        </button>
                      ) : (
                        <button class="timer-btn" onClick={() => onStartTimer(task)}>
                          {t('workMode.startTimer')}
                        </button>
                      )}
                    </div>
                  </article>
                ))
              )}
            </div>
          </section>
        ))}
      </div>
    </div>
  );
}

function formatDuration(ms: number): string {
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${sec}s`;
  return `${sec}s`;
}

function formatTime(unix: number): string {
  const d = new Date(unix * 1000);
  const y = d.getFullYear();
  const m = (d.getMonth() + 1).toString().padStart(2, '0');
  const day = d.getDate().toString().padStart(2, '0');
  return `${y}-${m}-${day}`;
}
