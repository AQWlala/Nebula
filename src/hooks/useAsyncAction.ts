/**
 * useAsyncAction — 防重复点击的异步 action hook。
 *
 * 用法:
 *   const { run, loading, error, data, reset } = useAsyncAction(fetchData);
 *   <button onClick={() => run(arg)} disabled={loading}>{loading ? '...' : 'Go'}</button>
 *
 * 设计要点:
 *  - useRef 存储 loading 状态,避免闭包陷阱(快速连点时拿到旧 loading)。
 *  - useState 仅用于触发 re-render,真正判定是否在跑的是 ref。
 *  - run 在上一个请求未完成时直接返回 undefined(被跳过),不排队、不并发。
 *  - error 捕获异常并存储;data 存储成功结果;reset 清空 error 与 data。
 */
import { useRef, useState, useCallback } from 'preact/hooks';

export function useAsyncAction<T, A extends any[]>(
  action: (...args: A) => Promise<T>,
): {
  run: (...args: A) => Promise<T | undefined>;
  loading: boolean;
  error: Error | null;
  data: T | null;
  reset: () => void;
} {
  // 真正的 loading 判定源:ref 同步更新,不受闭包影响。
  const loadingRef = useRef(false);
  // 触发 re-render 的镜像值。读 loadingRef.value 即可拿到最新状态。
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const [data, setData] = useState<T | null>(null);

  const run = useCallback(
    async (...args: A): Promise<T | undefined> => {
      // 防重复点击:上一个请求仍在进行中,直接跳过本次调用。
      if (loadingRef.current) return undefined;
      loadingRef.current = true;
      setLoading(true);
      setError(null);
      try {
        const result = await action(...args);
        setData(result);
        return result;
      } catch (e) {
        const err = e instanceof Error ? e : new Error(String(e));
        setError(err);
        throw err;
      } finally {
        loadingRef.current = false;
        setLoading(false);
      }
    },
    [action],
  );

  const reset = useCallback(() => {
    setError(null);
    setData(null);
  }, []);

  return { run, loading, error, data, reset };
}
