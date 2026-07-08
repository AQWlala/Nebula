/**
 * T-E-B-13: 知识卡片弹窗。
 *
 * 点击 ChatPanel 中 `[[xxx]]` wiki-link 后弹出,聚合展示:
 * 标题 / 定义 / 正文(markdown 渲染) / 关联实体 / 反向链接。
 *
 * **嵌套导航**:点击 related_entities 中的实体链接会在本弹窗内加载新卡片
 * (内部 slug state,不涉及父组件),最大嵌套深度 3(R3 风险控制)。
 *
 * **XSS 防护**(R1):正文经 `marked.parse` 解析后用 `DOMPurify.sanitize` 清理,
 * 防止 LLM 输出中的恶意脚本注入。`[[xxx]]` 链接预处理为 `<a class="wiki-link">`
 * 后再走 marked,最终 HTML 统一过 DOMPurify。
 *
 * 复用 global.css modal 类(modal-backdrop / modal / modal__header / modal__body)。
 */
import { useState, useEffect } from 'preact/hooks';
import { renderMarkdown } from '../utils/markdown';
import { nebulaAPI, type KnowledgeCard } from '../lib/tauri';
import { toast } from './Toast';
import { Modal } from './Modal';
import { t } from '../i18n';

interface KnowledgeCardDialogProps {
  slug: string | null;
  onClose: () => void;
}

/** R3: 最大嵌套深度(点击 related_entities 加载新卡片的次数)。 */
const MAX_NESTING_DEPTH = 3;

export function KnowledgeCardDialog({ slug, onClose }: KnowledgeCardDialogProps) {
  const [card, setCard] = useState<KnowledgeCard | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // 内部 slug state:支持点击 related_entities 嵌套加载新卡片。
  const [currentSlug, setCurrentSlug] = useState<string | null>(slug);
  // 嵌套深度(R3:限制最大 3 层,防止无限嵌套)。
  const [depth, setDepth] = useState(0);

  // 父组件传入新 slug 时重置内部状态(新弹窗会话)。
  useEffect(() => {
    setCurrentSlug(slug);
    setDepth(0);
  }, [slug]);

  // slug 变化时拉取 KnowledgeCard。
  useEffect(() => {
    if (!currentSlug) {
      setCard(null);
      return;
    }
    setLoading(true);
    setError(null);
    nebulaAPI
      .wikiGetCard(currentSlug)
      .then((c) => {
        setCard(c);
        setLoading(false);
      })
      .catch((e) => {
        const msg = String(e);
        setError(msg);
        setLoading(false);
        toast.error(t('knowledgeCardDialog.loadFailed'), msg);
      });
  }, [currentSlug]);

  const handleRelatedClick = (entitySlug: string) => {
    if (depth >= MAX_NESTING_DEPTH - 1) {
      toast.warning(
        t('knowledgeCardDialog.nestingLimit'),
        t('knowledgeCardDialog.nestingLimitBody', { count: MAX_NESTING_DEPTH })
      );
      return;
    }
    setCurrentSlug(entitySlug);
    setDepth((d) => d + 1);
  };

  const title = card ? card.note.title : t('knowledgeCardDialog.loadingTitle');

  return (
    <Modal open={!!slug} title={title} onClose={onClose} size="md">
      {loading && (
        <div
          style={{
            padding: '24px',
            textAlign: 'center',
            color: 'var(--text-secondary)',
          }}
        >
          {t('knowledgeCardDialog.loading')}
        </div>
      )}

      {error && !loading && (
        <div style={{ padding: '16px', color: 'var(--danger, #f44336)' }}>
          {t('knowledgeCardDialog.loadError', { error })}
        </div>
      )}

      {card && !loading && (
        <>
          {card.definition && (
            <div
              style={{
                padding: '8px 12px',
                marginBottom: '12px',
                background: 'var(--bg-tertiary, rgba(255,255,255,0.03))',
                borderLeft: '3px solid var(--accent-neon)',
                borderRadius: '4px',
                fontSize: '13px',
                color: 'var(--text-primary)',
              }}
            >
              {card.definition}
            </div>
          )}

          {/* 正文(markdown 渲染 + wiki-link 预处理 + DOMPurify 清理)。
              点击 .wiki-link 在本弹窗内嵌套加载新卡片。 */}
          <div
            class="msg-content"
            dangerouslySetInnerHTML={{ __html: renderMarkdown(card.body) }}
            onClick={(e) => {
              const target = e.target as HTMLElement;
              if (target.classList.contains('wiki-link')) {
                const s = target.dataset.slug;
                if (s) handleRelatedClick(s);
              }
            }}
          />

          {card.related_entities.length > 0 && (
            <div style={{ marginTop: '16px' }}>
              <h4
                style={{
                  fontSize: '13px',
                  color: 'var(--text-secondary)',
                  marginBottom: '6px',
                }}
              >
                {t('knowledgeCardDialog.relatedEntities')}
              </h4>
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: '6px' }}>
                {card.related_entities.map((e, i) => (
                  <a
                    key={`${e}-${i}`}
                    class="wiki-link"
                    data-slug={e}
                    onClick={(ev) => {
                      ev.preventDefault();
                      handleRelatedClick(e);
                    }}
                    style={{
                      cursor: 'pointer',
                      color: 'var(--accent-neon)',
                      textDecoration: 'underline',
                      fontSize: '12px',
                    }}
                  >
                    {e}
                  </a>
                ))}
              </div>
            </div>
          )}

          {card.backlinks.length > 0 && (
            <div style={{ marginTop: '12px' }}>
              <h4
                style={{
                  fontSize: '13px',
                  color: 'var(--text-secondary)',
                  marginBottom: '6px',
                }}
              >
                {t('knowledgeCardDialog.backlinks', { count: card.backlinks.length })}
              </h4>
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: '6px' }}>
                {card.backlinks.map((b, i) => (
                  <span
                    key={`${b}-${i}`}
                    style={{
                      padding: '2px 8px',
                      background: 'var(--bg-tertiary, rgba(255,255,255,0.05))',
                      borderRadius: '4px',
                      fontSize: '12px',
                      color: 'var(--text-secondary)',
                    }}
                  >
                    {b}
                  </span>
                ))}
              </div>
            </div>
          )}
        </>
      )}
    </Modal>
  );
}
