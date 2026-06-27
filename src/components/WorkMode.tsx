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
import { NineSnakeAPI, type WorkTask, type WorkTaskStatus } from '../lib/tauri';

const COLUMNS: { id: WorkTaskStatus; label: string; accent: string }[] = [
  { id: 'todo',  label: '待办', accent: '#7a8a9a' },
  { id: 'doing', label: '进行中', accent: '#ffb86b' },
  { id: 'done',  label: '已完成', accent: '#39d98a' },
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
    NineSnakeAPI.workActiveTimer().then(setActiveTimer).catch(() => undefined);
  }, []);

  const refresh = async () => {
    try {
      const all = await NineSnakeAPI.workListTasks(undefined, 200);
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
      const aiPri = await NineSnakeAPI.workRecommendPriority(newTitle, dueAt);
      const finalPri = Math.max(newPriority, aiPri);
      await NineSnakeAPI.workCreateTask({
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
      await NineSnakeAPI.workSetStatus(task.id, status);
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
          await NineSnakeAPI.workStopTimer();
        }
      }
      await NineSnakeAPI.workStartTimer(task.id);
      setActiveTimer(task.id);
      // 移动到 doing
      if (task.status !== 'doing') {
        await NineSnakeAPI.workSetStatus(task.id, 'doing');
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
        await NineSnakeAPI.workAddTime(task.id, elapsed);
      }
      await NineSnakeAPI.workStopTimer();
      setActiveTimer(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const onDelete = async (task: WorkTask) => {
    if (!confirm(`确认删除任务「${task.title}」？`)) return;
    try {
      await NineSnakeAPI.workDeleteTask(task.id);
      if (activeTimer === task.id) setActiveTimer(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const onSummarise = async () => {
    try {
      const out = await NineSnakeAPI.workSummariseMeeting(transcript);
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
          placeholder="+ 新任务标题"
          value={newTitle}
          onInput={(e) => setNewTitle((e.target as HTMLInputElement).value)}
          onKeyDown={(e) => { if (e.key === 'Enter') onCreate(); }}
        />
        <input
          class="work-new-desc"
          placeholder="（可选）描述"
          value={newDesc}
          onInput={(e) => setNewDesc((e.target as HTMLInputElement).value)}
        />
        <select
          class="work-new-priority"
          value={newPriority}
          onChange={(e) => setNewPriority(parseInt((e.target as HTMLSelectElement).value, 10))}
          title="最低优先级；AI 会自动上调"
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
          title="截止时间"
        />
        <button class="primary" onClick={onCreate}>添加</button>
        <div class="spacer" />
        <button class="ghost" onClick={() => setShowMeeting((v) => !v)}>
          {showMeeting ? '收起' : '展开'}会议纪要
        </button>
      </header>

      {showMeeting && (
        <section class="work-meeting">
          <textarea
            placeholder="粘贴会议转写文本（每行一句；- 开头的行视为 action item）"
            value={transcript}
            onInput={(e) => setTranscript((e.target as HTMLTextAreaElement).value)}
            rows={6}
          />
          <div class="row">
            <button onClick={onSummarise} class="primary">生成纪要</button>
            {meetingOut && (
              <>
                <span class="meeting-stat">决议 {meetingOut.decisions.length}</span>
                <span class="meeting-stat">行动 {meetingOut.actions.length}</span>
              </>
            )}
          </div>
          {meetingOut && (
            <div class="meeting-result">
              <div>
                <h4>决议</h4>
                <ol>
                  {meetingOut.decisions.map((d, i) => (<li key={i}>{d}</li>))}
                </ol>
              </div>
              <div>
                <h4>行动项</h4>
                <ul>
                  {meetingOut.actions.map((a, i) => (<li key={i}>{a}</li>))}
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
              const t = tasks.find((x) => x.id === id);
              if (t) onMove(t, col.id);
            }}
          >
            <header class="col-header" style={{ borderTop: `2px solid ${col.accent}` }}>
              <span>{col.label}</span>
              <span class="col-count">{grouped[col.id].length}</span>
            </header>
            <div class="col-body">
              {grouped[col.id].length === 0 ? (
                <p class="empty">—</p>
              ) : (
                grouped[col.id].map((t) => (
                  <article
                    key={t.id}
                    class={`task-card priority-${t.priority} ${activeTimer === t.id ? 'is-timed' : ''}`}
                    draggable
                    onDragStart={(e) => e.dataTransfer?.setData('text/plain', t.id)}
                  >
                    <div class="task-head">
                      <span class={`prio prio-${t.priority}`}>P{t.priority}</span>
                      <h4 class="task-title">{t.title}</h4>
                      <button class="task-del" onClick={() => onDelete(t)}>×</button>
                    </div>
                    {t.description && <p class="task-desc">{t.description}</p>}
                    <div class="task-foot">
                      {t.due_at && (
                        <span class="task-due">📅 {formatTime(t.due_at)}</span>
                      )}
                      <span class="task-time">⏱ {formatDuration(t.time_spent_ms)}</span>
                      <div class="spacer" />
                      {activeTimer === t.id ? (
                        <button class="timer-btn running" onClick={() => onStopTimer(t)}>
                          ⏹ 停止 ({formatDuration(Date.now() - tickStart)})
                        </button>
                      ) : (
                        <button class="timer-btn" onClick={() => onStartTimer(t)}>
                          ▶ 开始
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
