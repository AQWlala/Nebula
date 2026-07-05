/**
 * T-E-A-14: Arena A/B 测试主面板 — 创建对战 + 投票 + 排行榜。
 *
 * 流程:
 *   1. 用户输入 prompt + 选择 model_a / model_b
 *   2. 点击"创建对战"调用 `arenaCreateMatch`(后端 stub:响应为空,winner=NULL)
 *   3. 创建成功后展示"投票"按钮组(a / b / tie)—— 人工判定覆盖 winner
 *   4. 投票后刷新排行榜(`LeaderboardTable`)
 *
 * 后端 stub 说明:
 *   - `create_match` 当前不调用 LLM(response_a/b 为空),auto_score 为 None
 *   - 人工投票通过 `arenaVote` 触发 ELO 更新
 *   - 排行榜通过 `arenaLeaderboard` 拉取(按 ELO 降序)
 */
import { useEffect, useState } from 'preact/hooks';
import { nebulaAPI, type LeaderboardRow } from '../lib/tauri';
import { LeaderboardTable } from './LeaderboardTable';
import { toast } from './Toast';
import { t } from '../i18n';

const DEFAULT_MODEL_A = 'deepseek-chat';
const DEFAULT_MODEL_B = 'qwen2.5:7b';

type Winner = 'a' | 'b' | 'tie';

interface PendingMatch {
  matchId: string;
  prompt: string;
  modelA: string;
  modelB: string;
}

export function ArenaPanel() {
  const [prompt, setPrompt] = useState('');
  const [modelA, setModelA] = useState(DEFAULT_MODEL_A);
  const [modelB, setModelB] = useState(DEFAULT_MODEL_B);
  const [leaderboard, setLeaderboard] = useState<LeaderboardRow[]>([]);
  const [pending, setPending] = useState<PendingMatch | null>(null);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refreshLeaderboard = async () => {
    try {
      const lb = await nebulaAPI.arenaLeaderboard();
      setLeaderboard(lb);
    } catch (e) {
      // 排行榜刷新失败不阻塞 UI,仅记日志
      console.error('[ArenaPanel] refreshLeaderboard failed:', e);
    }
  };

  useEffect(() => {
    void refreshLeaderboard();
  }, []);

  const createMatch = async () => {
    if (!prompt.trim()) {
      setError(t('arena.promptRequired'));
      return;
    }
    if (modelA === modelB) {
      setError(t('arena.modelsMustDiffer'));
      return;
    }
    setError(null);
    setCreating(true);
    try {
      const matchId = await nebulaAPI.arenaCreateMatch(prompt, modelA, modelB);
      setPending({ matchId, prompt, modelA, modelB });
      toast.success(t('arena.matchCreated'));
      void refreshLeaderboard();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(t('arena.createFailed', { msg }));
      toast.error(t('arena.createFailed', { msg: '' }), msg);
    } finally {
      setCreating(false);
    }
  };

  const vote = async (winner: Winner) => {
    if (!pending) return;
    try {
      await nebulaAPI.arenaVote(pending.matchId, winner);
      const winnerLabel = winner === 'a' ? pending.modelA : winner === 'b' ? pending.modelB : t('arena.tie');
      toast.success(t('arena.voted', { winner: winnerLabel }));
      setPending(null);
      void refreshLeaderboard();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(t('arena.voteFailed', { msg }));
      toast.error(t('arena.voteFailed', { msg: '' }), msg);
    }
  };

  return (
    <div class="arena-panel" data-testid="arena-panel">
      <div class="arena-panel__header">
        <h2 class="arena-panel__title">{t('arena.title')}</h2>
        <button
          type="button"
          class="arena-panel__refresh"
          onClick={() => void refreshLeaderboard()}
          data-testid="arena-refresh-btn"
        >
          {t('arena.refreshLeaderboard')}
        </button>
      </div>

      <div class="arena-panel__form">
        <textarea
          class="arena-panel__prompt"
          placeholder={t('arena.promptPlaceholder')}
          value={prompt}
          onInput={(e) => setPrompt((e.target as HTMLTextAreaElement).value)}
          rows={4}
          data-testid="arena-prompt-input"
        />
        <div class="arena-panel__models">
          <label class="arena-panel__model-label">
            <span>{t('arena.modelA')}</span>
            <input
              type="text"
              class="arena-panel__model-input"
              value={modelA}
              onInput={(e) => setModelA((e.target as HTMLInputElement).value)}
              data-testid="arena-model-a-input"
            />
          </label>
          <span class="arena-panel__vs">{t('arena.vs')}</span>
          <label class="arena-panel__model-label">
            <span>{t('arena.modelB')}</span>
            <input
              type="text"
              class="arena-panel__model-input"
              value={modelB}
              onInput={(e) => setModelB((e.target as HTMLInputElement).value)}
              data-testid="arena-model-b-input"
            />
          </label>
        </div>
        <button
          type="button"
          class="arena-panel__create-btn"
          disabled={creating || !prompt.trim()}
          onClick={() => void createMatch()}
          data-testid="arena-create-btn"
        >
          {creating ? t('arena.creating') : t('arena.createMatch')}
        </button>
        {error && (
          <div class="arena-panel__error" data-testid="arena-error">
            {error}
          </div>
        )}
      </div>

      {pending && (
        <div class="arena-panel__vote" data-testid="arena-vote-section">
          <div class="arena-panel__vote-info">
            <span dangerouslySetInnerHTML={{ __html: t('arena.matchCreatedInfo', { id: pending.matchId.slice(0, 8) }) }} />
            <br />
            <strong>{pending.modelA}</strong> {t('arena.vs')} <strong>{pending.modelB}</strong>
            <br />
            <small>prompt: {pending.prompt.slice(0, 60)}{pending.prompt.length > 60 ? '...' : ''}</small>
          </div>
          <div class="arena-panel__vote-buttons">
            <button
              type="button"
              class="arena-panel__vote-btn arena-panel__vote-btn--a"
              onClick={() => void vote('a')}
              data-testid="arena-vote-a"
            >
              {t('arena.voteA')}
            </button>
            <button
              type="button"
              class="arena-panel__vote-btn arena-panel__vote-btn--tie"
              onClick={() => void vote('tie')}
              data-testid="arena-vote-tie"
            >
              {t('arena.voteTie')}
            </button>
            <button
              type="button"
              class="arena-panel__vote-btn arena-panel__vote-btn--b"
              onClick={() => void vote('b')}
              data-testid="arena-vote-b"
            >
              {t('arena.voteB')}
            </button>
          </div>
        </div>
      )}

      <div class="arena-panel__leaderboard">
        <h3 class="arena-panel__leaderboard-title">{t('arena.leaderboard')}</h3>
        <LeaderboardTable rows={leaderboard} />
      </div>
    </div>
  );
}
