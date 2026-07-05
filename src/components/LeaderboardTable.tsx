/**
 * T-E-A-14: Arena 排行榜表格 — 按 ELO 降序展示模型对战排名。
 *
 * 纯展示组件,接收 `LeaderboardRow[]` 数据,渲染表格:
 *   排名 | 模型 | ELO 分数 | 相对最高分差值
 *
 * 配色与 CreditsDashboard 的 BarChart 一致;首行(冠军)金色高亮。
 */
import type { LeaderboardRow } from '../lib/tauri';
import { t } from '../i18n';

interface LeaderboardTableProps {
  rows: LeaderboardRow[];
}

/** ELO 初始分(与后端 `arena::ELO_INIT` 对齐)。 */
const ELO_INIT = 1200;

function formatElo(elo: number): string {
  return elo.toFixed(1);
}

function formatDelta(delta: number): string {
  if (delta === 0) return '—';
  return delta > 0 ? `+${delta.toFixed(1)}` : delta.toFixed(1);
}

export function LeaderboardTable({ rows }: LeaderboardTableProps) {
  if (rows.length === 0) {
    return (
      <div class="arena-leaderboard__empty" data-testid="arena-leaderboard-empty">
        {t('leaderboardTable.empty')}
      </div>
    );
  }

  const topElo = rows[0]?.[1] ?? ELO_INIT;

  return (
    <table class="arena-leaderboard__table" data-testid="arena-leaderboard-table">
      <thead>
        <tr>
          <th class="arena-leaderboard__rank">#</th>
          <th class="arena-leaderboard__model">{t('leaderboardTable.model')}</th>
          <th class="arena-leaderboard__elo">ELO</th>
          <th class="arena-leaderboard__delta">Δ Top</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((row, idx) => {
          const [model, elo] = row;
          const delta = elo - topElo;
          const isChampion = idx === 0;
          return (
            <tr
              key={model}
              class={isChampion ? 'arena-leaderboard__row arena-leaderboard__row--champion' : 'arena-leaderboard__row'}
              data-testid={`arena-leaderboard-row-${idx}`}
            >
              <td class="arena-leaderboard__rank">{idx + 1}</td>
              <td class="arena-leaderboard__model">{model}</td>
              <td class="arena-leaderboard__elo">{formatElo(elo)}</td>
              <td class={delta < 0 ? 'arena-leaderboard__delta arena-leaderboard__delta--neg' : 'arena-leaderboard__delta'}>
                {formatDelta(delta)}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
