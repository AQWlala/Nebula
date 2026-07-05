/**
 * v1.0: command palette (⌘K / Ctrl-K).
 *
 * Fuzzy-searches a static list of commands plus recent
 * memories.  We deliberately do *not* use a global
 * event listener at module load time — the App owns the
 * keyboard handler so the palette is unit-testable.
 */
import Fuse from 'fuse.js';
import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { nebulaAPI } from '../lib/tauri';
import { t } from '../i18n';

export interface CommandItem {
  id: string;
  title: string;
  hint?: string;
  group: string;
  run: () => void | Promise<void>;
}

export function buildDefaultCommands(
  onClose: () => void,
  actions: {
    setMode: (m: 'chat' | 'swarm' | 'memory' | 'code' | 'skills') => void;
    setSubMode: (m: 'writing' | 'work' | 'code') => void;
    openSettings: () => void;
    triggerReflection: () => void;
  },
): CommandItem[] {
  return [
    { id: 'view.chat', title: t('command.view.chat'), group: t('command.group.view'), run: () => { actions.setMode('chat'); onClose(); } },
    { id: 'view.swarm', title: t('command.view.swarm'), group: t('command.group.view'), run: () => { actions.setMode('swarm'); onClose(); } },
    { id: 'view.memory', title: t('command.view.memory'), group: t('command.group.view'), run: () => { actions.setMode('memory'); onClose(); } },
    { id: 'view.code', title: t('command.view.code'), group: t('command.group.view'), run: () => { actions.setMode('code'); onClose(); } },
    { id: 'view.skills', title: t('command.view.skills'), group: t('command.group.view'), run: () => { actions.setMode('skills'); onClose(); } },
    { id: 'submode.writing', title: t('command.submode.writing'), group: t('command.group.submode'), run: () => { actions.setSubMode('writing'); onClose(); } },
    { id: 'submode.work', title: t('command.submode.work'), group: t('command.group.submode'), run: () => { actions.setSubMode('work'); onClose(); } },
    { id: 'submode.code', title: t('command.submode.code'), group: t('command.group.submode'), run: () => { actions.setSubMode('code'); onClose(); } },
    { id: 'action.reflect', title: t('command.action.reflect'), group: t('command.group.action'), run: () => { actions.triggerReflection(); onClose(); } },
    { id: 'action.open-settings', title: t('command.action.openSettings'), group: t('command.group.action'), run: () => { actions.openSettings(); onClose(); } },
  ];
}

export function CommandPalette({
  open,
  onClose,
  commands,
  extraItems,
}: {
  open: boolean;
  onClose: () => void;
  commands: CommandItem[];
  extraItems?: CommandItem[];
}) {
  const [q, setQ] = useState('');
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const all = useMemo(() => [...commands, ...(extraItems ?? [])], [commands, extraItems]);
  const fuse = useMemo(() => new Fuse(all, {
    keys: ['title', 'hint', 'group'],
    threshold: 0.4,
    ignoreLocation: true,
  }), [all]);
  const results = useMemo(() => {
    if (!q.trim()) return all.slice(0, 20);
    return fuse.search(q).map((r) => r.item).slice(0, 20);
  }, [q, all, fuse]);

  useEffect(() => {
    if (open) {
      setQ('');
      setActive(0);
      // Focus on next tick so the modal is mounted.
      setTimeout(() => inputRef.current?.focus(), 0);
    }
  }, [open]);

  if (!open) return null;

  function pick(item: CommandItem) {
    try {
      void item.run();
    } catch (e) {
      console.error('command failed', e);
    }
    onClose();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActive((a) => Math.min(results.length - 1, a + 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActive((a) => Math.max(0, a - 1));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const item = results[active];
      if (item) pick(item);
    }
  }

  return (
    <div class="command-overlay" role="dialog" aria-modal="true" onClick={onClose}>
      <div class="command-palette" onClick={(e) => e.stopPropagation()}>
        <div class="command-header">
          <span class="command-title">{t('command.title')}</span>
        </div>
        <input
          ref={inputRef}
          class="command-input"
          placeholder={t('command.placeholder')}
          value={q}
          onInput={(e) => { setQ(e.currentTarget.value); setActive(0); }}
          onKeyDown={onKey}
        />
        <ul class="command-list">
          {results.length === 0 && (
            <li class="command-empty">{t('command.empty')}</li>
          )}
          {results.map((it, i) => (
            <li
              key={it.id}
              class={`command-item ${i === active ? 'active' : ''}`}
              onMouseEnter={() => setActive(i)}
              onClick={() => pick(it)}
            >
              <span class="command-group">{it.group}</span>
              <span class="command-label">{it.title}</span>
              {it.hint && <span class="command-hint">{it.hint}</span>}
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

/** Convenience hook: install a global ⌘K listener. */
export function useCommandPaletteShortcut(handler: () => void) {
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && (e.key === 'k' || e.key === 'K')) {
        e.preventDefault();
        handler();
      }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [handler]);
}

export async function buildMemoryItems(limit = 10): Promise<CommandItem[]> {
  try {
    const mems = await nebulaAPI.memoryListRecent(limit);
    return mems.map((m) => ({
      id: `mem.${m.id}`,
      // P0#2: Memory.summary only has s50 / s150 / s500 / s2000.
      // Use s50 (the shortest summary) with a content fallback so
      // the list always renders without runtime TypeError.
      title: m.summary.s50 || m.content.slice(0, 50),
      hint: m.layer,
      group: t('command.group.memory'),
      run: () => {
        // Default: copy summary to clipboard so the user can paste
        // it into a chat.  Real v1.0 wire-up can navigate to the
        // memory inspector with this id.
        try {
          void navigator.clipboard.writeText(m.content);
        } catch {
          /* ignore */
        }
      },
    }));
  } catch {
    return [];
  }
}
