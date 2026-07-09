/**
 * v0.5: 文件树
 *
 * 渲染后端 editor::list_tree 返回的 FileEntry 列表。
 * 树形结构是用平铺的 path 字符串在客户端折叠出来的；
 * 不引入 react-arborist 以减小 bundle 体积。
 *
 * 交互：
 * - 点击文件 → 触发 onOpen(path)
 * - 点击目录 → 展开 / 收起
 */
import { useMemo, useState } from 'preact/hooks';
import { nebulaAPI, type FileEntry } from '../../lib/tauri';
import { t } from '../../i18n';

interface FileTreeProps {
  entries: FileEntry[];
  workspace: string;
  currentPath: string | null;
  onOpen: (path: string) => void;
  onRefresh: () => void;
}

interface TreeNode {
  name: string;
  path: string;
  isDir: boolean;
  children: TreeNode[];
}

function buildTree(entries: FileEntry[]): TreeNode[] {
  const root: TreeNode = { name: '', path: '', isDir: true, children: [] };
  for (const e of entries) {
    const parts = e.path.split('/').filter(Boolean);
    let cur = root;
    let acc = '';
    for (let i = 0; i < parts.length; i++) {
      const part = parts[i];
      acc = acc ? `${acc}/${part}` : part;
      const isLast = i === parts.length - 1;
      let child = cur.children.find((c) => c.name === part);
      if (!child) {
        child = {
          name: part,
          path: acc,
          isDir: isLast ? e.is_dir : true,
          children: [],
        };
        cur.children.push(child);
      } else if (isLast) {
        // Leaf could be a file or a dir entry; trust the latest.
        child.isDir = e.is_dir;
      }
      cur = child;
    }
  }
  // 排序：目录优先，按名
  const sort = (n: TreeNode) => {
    n.children.sort((a, b) => {
      if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
    n.children.forEach(sort);
  };
  sort(root);
  return root.children;
}

export function FileTree({ entries, workspace, currentPath, onOpen, onRefresh }: FileTreeProps) {
  const tree = useMemo(() => buildTree(entries), [entries]);
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set(['']));

  const toggle = (path: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  return (
    <div class="file-tree">
      <header class="ft-header">
        <span title={workspace} class="ft-root">
          {workspace}
        </span>
        <button class="ft-refresh" onClick={onRefresh} title={t('fileTree.refreshTitle')}>
          ↻
        </button>
      </header>
      <div class="ft-body">
        {tree.length === 0 ? (
          <p class="ft-empty">{t('fileTree.empty')}</p>
        ) : (
          tree.map((n) => (
            <Node
              key={n.path}
              node={n}
              depth={0}
              expanded={expanded}
              currentPath={currentPath}
              onOpen={onOpen}
              toggle={toggle}
            />
          ))
        )}
      </div>
    </div>
  );
}

interface NodeProps {
  node: TreeNode;
  depth: number;
  expanded: Set<string>;
  currentPath: string | null;
  onOpen: (path: string) => void;
  toggle: (path: string) => void;
}

function Node({ node, depth, expanded, currentPath, onOpen, toggle }: NodeProps) {
  const isOpen = expanded.has(node.path);
  const active = !node.isDir && currentPath === node.path;
  return (
    <>
      <div
        class={`ft-row ${active ? 'active' : ''} ${node.isDir ? 'is-dir' : 'is-file'}`}
        style={{ paddingLeft: `${depth * 14 + 6}px` }}
        onClick={() => (node.isDir ? toggle(node.path) : onOpen(node.path))}
      >
        <span class="ft-icon">{node.isDir ? (isOpen ? '📂' : '📁') : '📄'}</span>
        <span class="ft-name">{node.name}</span>
      </div>
      {node.isDir &&
        isOpen &&
        node.children.map((c) => (
          <Node
            key={c.path}
            node={c}
            depth={depth + 1}
            expanded={expanded}
            currentPath={currentPath}
            onOpen={onOpen}
            toggle={toggle}
          />
        ))}
    </>
  );
}

/** 重新加载文件树的 helper。 */
export async function refreshFileTree(setEntries: (e: FileEntry[]) => void, depth = 8) {
  try {
    const list = await nebulaAPI.editorList(depth);
    setEntries(list);
  } catch (e) {
    console.error('refreshFileTree failed:', e);
  }
}
