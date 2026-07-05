/**
 * v0.5: 集成终端
 *
 * 用 xterm.js 渲染一个真实 PTY 风格的终端。
 * 后端命令通过 os_shell_exec 转发（白名单 + 超时）。
 * 历史：本地保存最近 200 条命令。
 */
import { useEffect, useRef, useState } from 'preact/hooks';
import { Terminal as XTerm } from 'xterm';
import { FitAddon } from 'xterm-addon-fit';
import 'xterm/css/xterm.css';
import { nebulaAPI, type ShellOutput } from '../../lib/tauri';

const HISTORY_MAX = 200;
const PROMPT = '$ ';

export function Terminal() {
  const containerRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const bufRef = useRef<string>('');
  const historyRef = useRef<string[]>([]);
  const histIndexRef = useRef<number>(-1);
  const [cwd, setCwd] = useState<string>('~');

  useEffect(() => {
    if (!containerRef.current) return;
    const term = new XTerm({
      theme: {
        background: '#0d1117',
        foreground: '#c9d1d9',
        cursor: '#39d98a',
        selectionBackground: '#264f78',
      },
      fontFamily: 'Menlo, Consolas, monospace',
      fontSize: 12,
      cursorBlink: true,
      convertEol: true,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(containerRef.current);
    try { fit.fit(); } catch { /* container not ready */ }
    xtermRef.current = term;
    fitRef.current = fit;
    writePrompt();
    term.onData((data) => handleData(data));
    const onResize = () => { try { fit.fit(); } catch { /* */ } };
    window.addEventListener('resize', onResize);
    return () => {
      window.removeEventListener('resize', onResize);
      term.dispose();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const writePrompt = () => {
    xtermRef.current?.write(`\r\n${cwd} ${PROMPT}`);
    bufRef.current = '';
  };

  const handleData = async (data: string) => {
    const term = xtermRef.current;
    if (!term) return;
    for (const ch of data) {
      const code = ch.charCodeAt(0);
      if (code === 13) {
        // Enter
        const line = bufRef.current.trim();
        term.write('\r\n');
        if (line) {
          if (historyRef.current[historyRef.current.length - 1] !== line) {
            historyRef.current.push(line);
            if (historyRef.current.length > HISTORY_MAX) historyRef.current.shift();
          }
          histIndexRef.current = historyRef.current.length;
          await runCommand(line);
        }
        writePrompt();
      } else if (code === 127) {
        // Backspace
        if (bufRef.current.length > 0) {
          bufRef.current = bufRef.current.slice(0, -1);
          term.write('\b \b');
        }
      } else if (code === 27) {
        // Escape sequence (arrows) — handle lightly.
        // Minimal: ignore; full vt100 emulation is a v1.0 item.
        // We still need to consume the bytes so the buffer stays sane.
        continue;
      } else if (code === 3) {
        // Ctrl+C
        term.write('^C');
        writePrompt();
      } else if (code >= 32) {
        bufRef.current += ch;
        term.write(ch);
      }
    }
  };

  const runCommand = async (line: string) => {
    if (line === 'clear' || line === 'cls') {
      xtermRef.current?.clear();
      return;
    }
    if (line === 'pwd') {
      xtermRef.current?.writeln(cwd);
      return;
    }
    if (line.startsWith('cd ')) {
      const target = line.slice(3).trim();
      if (target === '~' || target === '') {
        setCwd('~');
      } else {
        setCwd(`${cwd === '~' ? '' : cwd}/${target}`);
      }
      return;
    }
    // Real command → backend
    try {
      const argv = line.split(/\s+/);
      const out = await nebulaAPI.osShellExec({ argv, timeout_ms: 30_000 });
      writeOutput(out);
    } catch (e) {
      xtermRef.current?.writeln(`\x1b[31m${String(e)}\x1b[0m`);
    }
  };

  const writeOutput = (out: ShellOutput) => {
    const term = xtermRef.current;
    if (!term) return;
    if (out.stdout) {
      term.write(out.stdout.replace(/\n/g, '\r\n'));
    }
    if (out.stderr) {
      term.write(`\x1b[33m${out.stderr.replace(/\n/g, '\r\n')}\x1b[0m`);
    }
    if (out.timed_out) {
      term.writeln(`\x1b[31m[timed out after ${out.elapsed_ms}ms]\x1b[0m`);
    } else if (out.exit_code !== 0) {
      term.writeln(`\x1b[31m[exit ${out.exit_code}]\x1b[0m`);
    }
  };

  return <div ref={containerRef} class="xterm-host" />;
}
