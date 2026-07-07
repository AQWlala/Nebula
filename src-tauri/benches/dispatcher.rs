//! M7b #93: ADR-003 性能回归基准(dispatch 延迟 + 流式延迟 + 并发限流)。
//!
//! 3 个 criterion 基准覆盖 UnifiedModelDispatcher 的核心热路径:
//!
//!   1. `dispatcher_construct`  — Dispatcher 构造开销(无网络)。
//!   2. `worktype_resolve`      — ModelPolicy::resolve() 路由决策开销(纯计算)。
//!   3. `dispatch_fail_fast`    — dispatch() 到死端口的失败快速返回延迟
//!      (验证断路器/Semaphore 限流不引入额外开销)。
//!
//! 注意:这些基准不依赖外部 LLM 服务。dispatch_fail_fast 使用 127.0.0.1:1
//! 死端口,mock OllamaClient 在 TCP 连接失败后立即返回 Err。基线对比的是
//! *失败路径* 的开销,而非 LLM 推理延迟(后者依赖硬件,不适合回归测试)。
//!
//! 运行:`cargo bench --bench dispatcher`
//! 对比基线:`cargo bench --bench dispatcher -- --save-baseline m7b`
//! 与基线对比:`cargo bench --bench dispatcher -- --baseline m7b`

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use nebula_lib::llm::{
    dispatcher::{ModelPolicy, UnifiedModelDispatcher, WorkType},
    LlmGateway, OllamaClient,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// 构造一个最小 mock LlmGateway(指向死端口,2s 超时快速失败)。
fn mock_gateway() -> Arc<LlmGateway> {
    let client = Arc::new(OllamaClient::new_with_timeout(
        "http://127.0.0.1:1",
        Duration::from_millis(500),
    ));
    Arc::new(LlmGateway::new(
        client, "mock", "ollama", None, None, None, None, None,
    ))
}

/// 构造一个最小 ModelPolicy(默认远端,无 override)。
fn mock_policy() -> ModelPolicy {
    ModelPolicy::new(
        "deepseek".to_string(),
        "deepseek-chat".to_string(),
        "ollama".to_string(),
        "qwen2.5:3b".to_string(),
        "qwen2.5:7b".to_string(),
        "qwen2.5:7b".to_string(),
        "qwen2.5:3b".to_string(),
        HashMap::new(),
    )
}

/// 构造一个 Dispatcher(无缓存,无 cost_tracker,max_local_concurrency=2)。
fn mock_dispatcher() -> Arc<UnifiedModelDispatcher> {
    Arc::new(UnifiedModelDispatcher::new(
        mock_gateway(),
        mock_policy(),
        None,
        None,
        2,
    ))
}

/// 基准 1: Dispatcher 构造开销。
///
/// 验证 UnifiedModelDispatcher::new 不引入意外开销
/// (CircuitBreaker/Semaphore/AtomicU8 初始化应为 O(1))。
fn bench_dispatcher_construct(c: &mut Criterion) {
    let gw = mock_gateway();
    let policy = mock_policy();

    c.bench_function("dispatcher_construct", |b| {
        b.iter(|| {
            let _d = UnifiedModelDispatcher::new(gw.clone(), policy.clone(), None, None, 2);
            black_box(());
        });
    });
}

/// 基准 2: ModelPolicy::resolve() 路由决策开销。
///
/// 纯计算路径(无网络),验证 7 个 WorkType 的 resolve 都是 O(1) HashMap 查询。
/// 这是 dispatch() 的热路径前缀,任何回归都会被捕获。
fn bench_worktype_resolve(c: &mut Criterion) {
    let policy = mock_policy();

    c.bench_function("worktype_resolve_all_seven", |b| {
        b.iter(|| {
            // 全部 7 个 WorkType 依次 resolve
            let r1 = policy.resolve(WorkType::Chat);
            let r2 = policy.resolve(WorkType::SwarmWorker);
            let r3 = policy.resolve(WorkType::SwarmSynthesize);
            let r4 = policy.resolve(WorkType::MasterTask);
            let r5 = policy.resolve(WorkType::Evolution);
            let r6 = policy.resolve(WorkType::SoulCompile);
            let r7 = policy.resolve(WorkType::Classifier);
            black_box((r1, r2, r3, r4, r5, r6, r7));
        });
    });
}

/// 基准 3: dispatch() 失败快速返回延迟。
///
/// 使用死端口(127.0.0.1:1) + 500ms 超时,mock OllamaClient 在 TCP 连接
/// 失败后立即返回 Err。基线对比的是 *失败路径* 开销:
/// - resolve() 路由决策
/// - dispatch_local() 路径(CircuitBreaker check + Semaphore acquire + OllamaClient 调用)
/// - OllamaClient 网络错误捕获
///
/// **预期**:Evolution(is_local_only=true)走 dispatch_local → OllamaClient
/// 直连死端口 → TCP 连接失败 → 快速返回 Err。500ms 超时是上限,实际应更快。
///
/// **回归信号**:如果某次改动引入了意外阻塞(如同步锁未释放、Semaphore
/// 配置错误),这个基准的时间会显著上升。
fn bench_dispatch_fail_fast(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let dispatcher = mock_dispatcher();

    c.bench_function("dispatch_fail_fast_local", |b| {
        b.iter(|| {
            // Evolution 强制本地路由(is_local_only=true),走 dispatch_local
            // → OllamaClient 直连死端口 → TCP 失败 → 快速 Err。
            let result = rt.block_on(async {
                dispatcher
                    .dispatch(
                        WorkType::Evolution,
                        vec![nebula_lib::llm::ChatMessage::user("hi")],
                    )
                    .await
            });
            // 确实是 Err(网络失败),不是 panic
            assert!(result.is_err(), "expected network error from dead port");
            let _ = black_box(result);
        });
    });
}

criterion_group!(
    benches,
    bench_dispatcher_construct,
    bench_worktype_resolve,
    bench_dispatch_fail_fast,
);
criterion_main!(benches);
