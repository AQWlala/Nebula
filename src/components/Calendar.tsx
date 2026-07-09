/**
 * T-E-S-58: Calendar — 日历组件。
 *
 * ## 功能
 * - 月视图:7 列网格,展示当月 6 周日期格子,事件按类型颜色编码显示
 * - 周视图:横向时间轴(7 天列 × 24 小时行),事件按时段绝对定位
 * - 事件显示:task / reminder / loop / custom 四类,支持自定义 color 覆盖
 * - 日期选择:点击日期格子触发 onDateSelect
 * - 前/后导航:月视图按月切换,周视图按周切换
 * - 今天按钮:一键回到今天
 * - 月/周视图切换
 *
 * ## i18n 策略
 * 不修改全局 src/i18n/*.json(避免并发冲突),沿用 workflow/i18n.ts 的模式:
 * 本模块自带中英字典 + 读取 currentLocale 信号实现响应式双语切换。
 *
 * ## 依赖
 * 纯 Preact hooks + 原生 Date API,不引入额外依赖。
 */
import { useState, useMemo, useCallback, useEffect } from 'preact/hooks';
import { currentLocale } from '../i18n';
import type { Locale } from '../i18n';

/** 日历事件。 */
export interface CalendarEvent {
  id: string;
  title: string;
  start: Date;
  end: Date;
  type: 'task' | 'reminder' | 'loop' | 'custom';
  color?: string;
}

/** Calendar 组件属性。 */
export interface CalendarProps {
  events: CalendarEvent[];
  view?: 'month' | 'week';
  selectedDate?: Date;
  onDateSelect?: (date: Date) => void;
  onEventClick?: (event: CalendarEvent) => void;
  onNavigate?: (date: Date, view: 'month' | 'week') => void;
}

/** 事件类型 → 主题色 + 背景色。 */
const EVENT_THEME: Record<
  CalendarEvent['type'],
  { color: string; bg: string }
> = {
  task: { color: '#3b82f6', bg: 'rgba(59,130,246,0.15)' },
  reminder: { color: '#f59e0b', bg: 'rgba(245,158,11,0.15)' },
  loop: { color: '#10b981', bg: 'rgba(16,185,129,0.15)' },
  custom: { color: '#8b5cf6', bg: 'rgba(139,92,246,0.15)' },
};

/** 月视图每个格子最多直接展示的事件数,超出折叠为「+N」。 */
const MAX_EVENTS_PER_CELL = 3;
/** 周视图每小时高度(px)。 */
const HOUR_HEIGHT = 44;

// ---- 日期工具(原生 Date API) ----

/** 返回某天的 00:00:00 本地时间。 */
function startOfDay(d: Date): Date {
  const r = new Date(d);
  r.setHours(0, 0, 0, 0);
  return r;
}

/** 返回 n 天后的新 Date(不修改原对象)。 */
function addDays(d: Date, n: number): Date {
  const r = new Date(d);
  r.setDate(r.getDate() + n);
  return r;
}

/** 返回 n 月后的新 Date,自动 clamp 到目标月的最后一天(避免 31 号溢出)。 */
function addMonths(d: Date, n: number): Date {
  const r = new Date(d);
  const day = r.getDate();
  r.setDate(1); // 先置 1,避免 setMonth 溢出
  r.setMonth(r.getMonth() + n);
  const lastDay = new Date(r.getFullYear(), r.getMonth() + 1, 0).getDate();
  r.setDate(Math.min(day, lastDay));
  return r;
}

/** 返回所在周的周一(Monday-based week)。 */
function startOfWeek(d: Date): Date {
  const day = d.getDay(); // 0=Sun, 1=Mon, ..., 6=Sat
  const offset = (day + 6) % 7; // 距周一的天数
  return startOfDay(addDays(d, -offset));
}

/** 判断两个 Date 是否同一天。 */
function isSameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

/** 判断两个 Date 是否同年同月。 */
function isSameMonth(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth();
}

/** 生成月视图 42 天网格(6 周 × 7 天,从包含当月 1 号的那一周周一开始)。 */
function getMonthGrid(d: Date): Date[] {
  const first = new Date(d.getFullYear(), d.getMonth(), 1);
  const gridStart = startOfWeek(first);
  return Array.from({ length: 42 }, (_, i) => addDays(gridStart, i));
}

/** 生成周视图 7 天(从所在周的周一开始)。 */
function getWeekDays(d: Date): Date[] {
  const start = startOfWeek(d);
  return Array.from({ length: 7 }, (_, i) => addDays(start, i));
}

/** 返回与某天有重叠的事件,按开始时间排序。 */
function getEventsOnDay(events: CalendarEvent[], day: Date): CalendarEvent[] {
  const dayStart = startOfDay(day);
  const dayEnd = addDays(dayStart, 1);
  return events
    .filter((e) => e.start < dayEnd && e.end > dayStart)
    .sort((a, b) => a.start.getTime() - b.start.getTime());
}

/** 将事件裁剪到某一天内,返回相对当天 00:00 的 top(分钟)与 duration(分钟)。 */
function clipToDay(e: CalendarEvent, day: Date): { topMin: number; durMin: number } {
  const dayStart = startOfDay(day);
  const dayEnd = addDays(dayStart, 1);
  const visStart = e.start > dayStart ? e.start : dayStart;
  const visEnd = e.end < dayEnd ? e.end : dayEnd;
  const topMin = (visStart.getTime() - dayStart.getTime()) / 60000;
  const durMin = (visEnd.getTime() - visStart.getTime()) / 60000;
  return { topMin, durMin };
}

/** 生成日期 testid 用的 key:YYYY-MM-DD。 */
function dayKey(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(
    d.getDate()
  ).padStart(2, '0')}`;
}

/** 格式化 HH:MM。 */
function formatTime(d: Date): string {
  return `${String(d.getHours()).padStart(2, '0')}:${String(
    d.getMinutes()
  ).padStart(2, '0')}`;
}

// ---- 本地双语字符串 ----

interface CalendarStrings {
  title: string;
  today: string;
  monthView: string;
  weekView: string;
  prev: string;
  next: string;
  more: string;
  noEvents: string;
  weekdays: string[];
}

const EN: CalendarStrings = {
  title: 'Calendar',
  today: 'Today',
  monthView: 'Month',
  weekView: 'Week',
  prev: 'Previous',
  next: 'Next',
  more: '+{n} more',
  noEvents: 'No events',
  weekdays: ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'],
};

const ZH: CalendarStrings = {
  title: '日历',
  today: '今天',
  monthView: '月',
  weekView: '周',
  prev: '上一个',
  next: '下一个',
  more: '还有 {n} 个',
  noEvents: '暂无事件',
  weekdays: ['一', '二', '三', '四', '五', '六', '日'],
};

const DICTS: Record<Locale, CalendarStrings> = {
  'en-US': EN,
  'zh-CN': ZH,
};

/** 获取当前语言的日历字符串(读取 currentLocale.value 建立信号订阅)。 */
function useStrings(): CalendarStrings {
  // 读取 .value 以订阅 locale 变化,使组件在语言切换时自动重渲染。
  return DICTS[currentLocale.value] ?? EN;
}

/** 格式化 {n} 占位符。 */
function fmt(s: string, n: number): string {
  return s.replace('{n}', String(n));
}

/** 格式化月视图标题:EN → "January 2025",ZH → "2025年1月"。 */
function monthTitle(d: Date, locale: Locale): string {
  return locale === 'zh-CN'
    ? `${d.getFullYear()}年${d.getMonth() + 1}月`
    : `${monthNameEN(d.getMonth())} ${d.getFullYear()}`;
}

/** 英文月份全名。 */
function monthNameEN(m: number): string {
  return [
    'January', 'February', 'March', 'April', 'May', 'June',
    'July', 'August', 'September', 'October', 'November', 'December',
  ][m];
}

/** 英文月份缩写。 */
function monthShortEN(m: number): string {
  return monthNameEN(m).slice(0, 3);
}

/** 格式化周视图标题:EN → "Jan 6 – Jan 12, 2025",ZH → "1月6日 – 1月12日"。 */
function weekTitle(days: Date[], locale: Locale): string {
  const a = days[0];
  const b = days[6];
  if (locale === 'zh-CN') {
    return `${a.getMonth() + 1}月${a.getDate()}日 – ${b.getMonth() + 1}月${b.getDate()}日`;
  }
  return `${monthShortEN(a.getMonth())} ${a.getDate()} – ${monthShortEN(b.getMonth())} ${b.getDate()}, ${b.getFullYear()}`;
}

// ---- 主组件 ----

export function Calendar(props: CalendarProps) {
  const s = useStrings();
  const today = useMemo(() => new Date(), []);

  const [viewMode, setViewMode] = useState<'month' | 'week'>(props.view ?? 'month');
  const [cursor, setCursor] = useState<Date>(props.selectedDate ?? today);
  const [selected, setSelected] = useState<Date>(props.selectedDate ?? today);

  // 外部 prop 变化时同步内部状态。
  useEffect(() => {
    if (props.view) setViewMode(props.view);
  }, [props.view]);
  useEffect(() => {
    if (props.selectedDate) {
      setCursor(props.selectedDate);
      setSelected(props.selectedDate);
    }
  }, [props.selectedDate]);

  const monthDays = useMemo(() => getMonthGrid(cursor), [cursor]);
  const weekDays = useMemo(() => getWeekDays(cursor), [cursor]);

  // ---- 导航 ----

  const goPrev = useCallback(() => {
    const next = viewMode === 'month' ? addMonths(cursor, -1) : addDays(startOfWeek(cursor), -7);
    setCursor(next);
    props.onNavigate?.(next, viewMode);
  }, [cursor, viewMode, props]);

  const goNext = useCallback(() => {
    const next = viewMode === 'month' ? addMonths(cursor, 1) : addDays(startOfWeek(cursor), 7);
    setCursor(next);
    props.onNavigate?.(next, viewMode);
  }, [cursor, viewMode, props]);

  const goToday = useCallback(() => {
    const now = new Date();
    setCursor(now);
    props.onNavigate?.(now, viewMode);
  }, [viewMode, props]);

  const switchView = useCallback(
    (v: 'month' | 'week') => {
      setViewMode(v);
      props.onNavigate?.(cursor, v);
    },
    [cursor, props]
  );

  const handleDateClick = useCallback(
    (d: Date) => {
      setSelected(d);
      props.onDateSelect?.(d);
    },
    [props]
  );

  const handleEventClick = useCallback(
    (e: CalendarEvent, ev: MouseEvent) => {
      ev.stopPropagation();
      props.onEventClick?.(e);
    },
    [props]
  );

  const headerLabel = viewMode === 'month' ? monthTitle(cursor, currentLocale.value) : weekTitle(weekDays, currentLocale.value);

  // ---- 渲染 ----

  return (
    <div
      data-testid="calendar-root"
      style={{
        border: '1px solid var(--border, #e5e7eb)',
        borderRadius: 8,
        background: 'var(--bg, #fff)',
        color: 'var(--text, #111)',
        fontFamily: 'system-ui, sans-serif',
        fontSize: 13,
        overflow: 'hidden',
      }}
    >
      {/* 工具栏 */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '8px 12px',
          borderBottom: '1px solid var(--border, #e5e7eb)',
          background: 'var(--bg-elevated, #f9fafb)',
        }}
      >
        <span data-testid="calendar-title" style={{ fontWeight: 600, flex: 1 }}>
          {headerLabel}
        </span>
        <button
          data-testid="calendar-prev-btn"
          onClick={goPrev}
          title={s.prev}
          style={btnStyle}
        >
          ‹
        </button>
        <button
          data-testid="calendar-today-btn"
          onClick={goToday}
          style={btnStyle}
        >
          {s.today}
        </button>
        <button
          data-testid="calendar-next-btn"
          onClick={goNext}
          title={s.next}
          style={btnStyle}
        >
          ›
        </button>
        <span style={{ width: 1, alignSelf: 'stretch', background: 'var(--border, #e5e7eb)', margin: '0 4px' }} />
        <button
          data-testid="calendar-view-month-btn"
          onClick={() => switchView('month')}
          style={viewMode === 'month' ? btnStyleActive : btnStyle}
        >
          {s.monthView}
        </button>
        <button
          data-testid="calendar-view-week-btn"
          onClick={() => switchView('week')}
          style={viewMode === 'week' ? btnStyleActive : btnStyle}
        >
          {s.weekView}
        </button>
      </div>

      {viewMode === 'month' ? (
        <MonthGrid
          days={monthDays}
          cursor={cursor}
          selected={selected}
          today={today}
          events={props.events}
          strings={s}
          onDateClick={handleDateClick}
          onEventClick={handleEventClick}
        />
      ) : (
        <WeekGrid
          days={weekDays}
          selected={selected}
          today={today}
          events={props.events}
          strings={s}
          onDateClick={handleDateClick}
          onEventClick={handleEventClick}
        />
      )}
    </div>
  );
}

// ---- 按钮样式 ----

const btnStyle = {
  border: '1px solid var(--border, #e5e7eb)',
  background: 'var(--bg, #fff)',
  color: 'inherit',
  padding: '4px 10px',
  borderRadius: 6,
  cursor: 'pointer',
  fontSize: 13,
};

const btnStyleActive = {
  ...btnStyle,
  background: 'var(--accent, #3b82f6)',
  color: '#fff',
  borderColor: 'var(--accent, #3b82f6)',
};

// ---- 月视图子组件 ----

interface MonthGridProps {
  days: Date[];
  cursor: Date;
  selected: Date;
  today: Date;
  events: CalendarEvent[];
  strings: CalendarStrings;
  onDateClick: (d: Date) => void;
  onEventClick: (e: CalendarEvent, ev: MouseEvent) => void;
}

function MonthGrid({
  days,
  cursor,
  selected,
  today,
  events,
  strings,
  onDateClick,
  onEventClick,
}: MonthGridProps) {
  return (
    <div data-testid="calendar-month-grid">
      {/* 星期表头 */}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(7, 1fr)', borderBottom: '1px solid var(--border, #e5e7eb)' }}>
        {strings.weekdays.map((wd, i) => (
          <div
            key={i}
            data-testid={`calendar-weekday-${i}`}
            style={{
              padding: '6px 4px',
              textAlign: 'center',
              fontWeight: 600,
              fontSize: 12,
              color: 'var(--text-secondary, #6b7280)',
            }}
          >
            {wd}
          </div>
        ))}
      </div>
      {/* 日期格子 */}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(7, 1fr)' }}>
        {days.map((d) => {
          const inMonth = isSameMonth(d, cursor);
          const isToday = isSameDay(d, today);
          const isSelected = isSameDay(d, selected);
          const dayEvents = getEventsOnDay(events, d);
          const visible = dayEvents.slice(0, MAX_EVENTS_PER_CELL);
          const hidden = dayEvents.length - visible.length;
          return (
            <div
              key={dayKey(d)}
              data-testid={`calendar-day-${dayKey(d)}`}
              data-selected={isSelected ? 'true' : 'false'}
              data-in-month={inMonth ? 'true' : 'false'}
              onClick={() => onDateClick(d)}
              style={{
                minHeight: 88,
                padding: 4,
                borderTop: '1px solid var(--border, #e5e7eb)',
                borderLeft: '1px solid var(--border, #e5e7eb)',
                cursor: 'pointer',
                opacity: inMonth ? 1 : 0.4,
                background: isSelected
                  ? 'rgba(59,130,246,0.08)'
                  : isToday
                    ? 'rgba(59,130,246,0.04)'
                    : 'transparent',
              }}
            >
              <div
                style={{
                  display: 'inline-flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  width: 22,
                  height: 22,
                  borderRadius: '50%',
                  fontSize: 12,
                  fontWeight: isToday ? 700 : 400,
                  background: isToday ? 'var(--accent, #3b82f6)' : 'transparent',
                  color: isToday ? '#fff' : 'inherit',
                }}
              >
                {d.getDate()}
              </div>
              {visible.map((e) => {
                const theme = EVENT_THEME[e.type];
                const color = e.color ?? theme.color;
                return (
                  <div
                    key={e.id}
                    data-testid={`calendar-event-${e.id}`}
                    data-event-type={e.type}
                    data-color={color}
                    onClick={(ev) => onEventClick(e, ev as unknown as MouseEvent)}
                    style={{
                      marginTop: 2,
                      padding: '1px 4px',
                      borderRadius: 3,
                      fontSize: 11,
                      whiteSpace: 'nowrap',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      borderLeft: `3px solid ${color}`,
                      background: theme.bg,
                      color: color,
                    }}
                  >
                    {e.title}
                  </div>
                );
              })}
              {hidden > 0 && (
                <div
                  data-testid={`calendar-more-${dayKey(d)}`}
                  style={{ fontSize: 11, color: 'var(--text-secondary, #6b7280)', marginTop: 2 }}
                >
                  {fmt(strings.more, hidden)}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ---- 周视图子组件 ----

interface WeekGridProps {
  days: Date[];
  selected: Date;
  today: Date;
  events: CalendarEvent[];
  strings: CalendarStrings;
  onDateClick: (d: Date) => void;
  onEventClick: (e: CalendarEvent, ev: MouseEvent) => void;
}

function WeekGrid({
  days,
  selected,
  today,
  events,
  strings,
  onDateClick,
  onEventClick,
}: WeekGridProps) {
  const bodyHeight = 24 * HOUR_HEIGHT;
  return (
    <div data-testid="calendar-week-grid" style={{ display: 'flex', flexDirection: 'column' }}>
      {/* 日期表头 */}
      <div style={{ display: 'flex', borderBottom: '1px solid var(--border, #e5e7eb)' }}>
        <div style={{ width: 48, flexShrink: 0 }} />
        {days.map((d, i) => {
          const isToday = isSameDay(d, today);
          const isSelected = isSameDay(d, selected);
          return (
            <div
              key={dayKey(d)}
              data-testid={`calendar-weekday-${i}`}
              onClick={() => onDateClick(d)}
              style={{
                flex: 1,
                padding: '6px 4px',
                textAlign: 'center',
                cursor: 'pointer',
                background: isSelected ? 'rgba(59,130,246,0.08)' : 'transparent',
              }}
            >
              <div style={{ fontSize: 12, color: 'var(--text-secondary, #6b7280)' }}>
                {strings.weekdays[i]}
              </div>
              <div
                style={{
                  display: 'inline-flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  width: 24,
                  height: 24,
                  borderRadius: '50%',
                  fontWeight: isToday ? 700 : 400,
                  background: isToday ? 'var(--accent, #3b82f6)' : 'transparent',
                  color: isToday ? '#fff' : 'inherit',
                }}
              >
                {d.getDate()}
              </div>
            </div>
          );
        })}
      </div>
      {/* 时间轴主体 */}
      <div style={{ display: 'flex', maxHeight: 440, overflowY: 'auto' }}>
        {/* 小时刻度列 */}
        <div style={{ width: 48, flexShrink: 0 }} data-testid="calendar-week-hours">
          {Array.from({ length: 24 }, (_, h) => (
            <div
              key={h}
              style={{
                height: HOUR_HEIGHT,
                fontSize: 10,
                color: 'var(--text-secondary, #6b7280)',
                textAlign: 'right',
                paddingRight: 4,
                borderBottom: '1px solid var(--border, #e5e7eb)',
              }}
            >
              {h === 0 ? '' : `${String(h).padStart(2, '0')}:00`}
            </div>
          ))}
        </div>
        {/* 7 天列 */}
        <div style={{ display: 'flex', flex: 1, position: 'relative' }}>
          {days.map((d) => {
            const dayEvents = getEventsOnDay(events, d);
            return (
              <div
                key={dayKey(d)}
                data-testid={`calendar-day-${dayKey(d)}`}
                onClick={() => onDateClick(d)}
                style={{
                  flex: 1,
                  position: 'relative',
                  height: bodyHeight,
                  borderLeft: '1px solid var(--border, #e5e7eb)',
                  cursor: 'pointer',
                }}
              >
                {/* 小时网格线 */}
                {Array.from({ length: 24 }, (_, h) => (
                  <div
                    key={h}
                    style={{
                      position: 'absolute',
                      left: 0,
                      right: 0,
                      top: h * HOUR_HEIGHT,
                      height: HOUR_HEIGHT,
                      borderBottom: '1px solid var(--border, #e5e7eb)',
                    }}
                  />
                ))}
                {/* 事件块 */}
                {dayEvents.map((e) => {
                  const { topMin, durMin } = clipToDay(e, d);
                  const theme = EVENT_THEME[e.type];
                  const color = e.color ?? theme.color;
                  const top = (topMin / 60) * HOUR_HEIGHT;
                  const height = Math.max(18, (durMin / 60) * HOUR_HEIGHT - 2);
                  return (
                    <div
                      key={e.id}
                      data-testid={`calendar-event-${e.id}`}
                      data-event-type={e.type}
                      data-color={color}
                      onClick={(ev) => onEventClick(e, ev as unknown as MouseEvent)}
                      style={{
                        position: 'absolute',
                        left: 2,
                        right: 2,
                        top,
                        height,
                        padding: '1px 4px',
                        borderRadius: 3,
                        fontSize: 11,
                        overflow: 'hidden',
                        borderLeft: `3px solid ${color}`,
                        background: theme.bg,
                        color: color,
                        boxSizing: 'border-box',
                      }}
                    >
                      <div style={{ fontWeight: 600, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                        {e.title}
                      </div>
                      <div style={{ fontSize: 10, opacity: 0.85 }}>
                        {formatTime(e.start)}–{formatTime(e.end)}
                      </div>
                    </div>
                  );
                })}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
