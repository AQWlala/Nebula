/**
 * T-E-S-58: Calendar 前端测试。
 *
 * 覆盖:
 * - 渲染:月/周视图默认与切换 / 标题 / 星期表头 / 今天按钮
 * - 导航:月视图前/后/今天 / 周视图前/后 / onNavigate 回调
 * - 交互:点击日期触发 onDateSelect / 点击事件触发 onEventClick
 * - 事件:按类型颜色编码 / 自定义 color 覆盖 / 事件落在正确日期格 /
 *   超出上限折叠为「+N」/ 周视图显示时段
 * - 选中态高亮 / 月外日期变暗
 * - i18n:中英双语切换
 */
import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/preact';
import { setLocale } from '../../i18n';
import { Calendar, type CalendarEvent } from '../Calendar';

/** 英文月份全名(用于断言月视图标题)。 */
const MONTHS_EN = [
  'January', 'February', 'March', 'April', 'May', 'June',
  'July', 'August', 'September', 'October', 'November', 'December',
];

/** 构造一个事件,默认落在 2025-01-15 10:00–11:00。 */
function makeEvent(overrides: Partial<CalendarEvent> = {}): CalendarEvent {
  const start = overrides.start ?? new Date(2025, 0, 15, 10, 0);
  const end = overrides.end ?? new Date(start.getTime() + 60 * 60 * 1000);
  const base: CalendarEvent = {
    id: overrides.id ?? 'evt-1',
    title: overrides.title ?? 'Event',
    start,
    end,
    type: overrides.type ?? 'task',
  };
  if (overrides.color !== undefined) base.color = overrides.color;
  return base;
}

/** 2025-01-15(周三)。 */
const JAN15 = new Date(2025, 0, 15);

beforeEach(() => {
  setLocale('en-US');
});

afterEach(() => {
  cleanup();
});

describe('Calendar — 渲染与视图切换', () => {
  it('renders_month_view_by_default', () => {
    const { queryByTestId } = render(<Calendar events={[]} selectedDate={JAN15} />);
    expect(queryByTestId('calendar-month-grid')).toBeTruthy();
    expect(queryByTestId('calendar-week-grid')).toBeFalsy();
  });

  it('renders_week_view_when_view_prop_is_week', () => {
    const { queryByTestId } = render(
      <Calendar events={[]} view="week" selectedDate={JAN15} />
    );
    expect(queryByTestId('calendar-week-grid')).toBeTruthy();
    expect(queryByTestId('calendar-month-grid')).toBeFalsy();
  });

  it('renders_today_button_and_navigation_buttons', () => {
    const { getByTestId } = render(<Calendar events={[]} selectedDate={JAN15} />);
    expect(getByTestId('calendar-today-btn').textContent).toBe('Today');
    expect(getByTestId('calendar-prev-btn')).toBeTruthy();
    expect(getByTestId('calendar-next-btn')).toBeTruthy();
  });

  it('renders_seven_weekday_headers', () => {
    const { getByTestId } = render(<Calendar events={[]} selectedDate={JAN15} />);
    for (let i = 0; i < 7; i++) {
      expect(getByTestId(`calendar-weekday-${i}`)).toBeTruthy();
    }
    expect(getByTestId('calendar-weekday-0').textContent).toContain('Mon');
    expect(getByTestId('calendar-weekday-6').textContent).toContain('Sun');
  });

  it('view_switcher_toggles_from_month_to_week', () => {
    const { getByTestId, queryByTestId } = render(
      <Calendar events={[]} selectedDate={JAN15} />
    );
    fireEvent.click(getByTestId('calendar-view-week-btn'));
    expect(queryByTestId('calendar-week-grid')).toBeTruthy();
    expect(queryByTestId('calendar-month-grid')).toBeFalsy();
  });

  it('view_switcher_toggles_from_week_to_month', () => {
    const { getByTestId, queryByTestId } = render(
      <Calendar events={[]} view="week" selectedDate={JAN15} />
    );
    fireEvent.click(getByTestId('calendar-view-month-btn'));
    expect(queryByTestId('calendar-month-grid')).toBeTruthy();
    expect(queryByTestId('calendar-week-grid')).toBeFalsy();
  });
});

describe('Calendar — 导航', () => {
  it('prev_button_navigates_to_previous_month_in_month_view', () => {
    const { getByTestId } = render(<Calendar events={[]} selectedDate={JAN15} />);
    expect(getByTestId('calendar-title').textContent).toContain('January');
    fireEvent.click(getByTestId('calendar-prev-btn'));
    expect(getByTestId('calendar-title').textContent).toContain('December');
    expect(getByTestId('calendar-title').textContent).toContain('2024');
  });

  it('next_button_navigates_to_next_month_in_month_view', () => {
    const { getByTestId } = render(<Calendar events={[]} selectedDate={JAN15} />);
    fireEvent.click(getByTestId('calendar-next-btn'));
    expect(getByTestId('calendar-title').textContent).toContain('February');
    expect(getByTestId('calendar-title').textContent).toContain('2025');
  });

  it('today_button_navigates_to_current_month', () => {
    const { getByTestId } = render(<Calendar events={[]} selectedDate={JAN15} />);
    fireEvent.click(getByTestId('calendar-today-btn'));
    const now = new Date();
    expect(getByTestId('calendar-title').textContent).toContain(MONTHS_EN[now.getMonth()]);
    expect(getByTestId('calendar-title').textContent).toContain(String(now.getFullYear()));
  });

  it('prev_button_navigates_to_previous_week_in_week_view', () => {
    // 2025-01-15 是周三 → 所在周为 Jan 13–19
    const { getByTestId } = render(
      <Calendar events={[]} view="week" selectedDate={JAN15} />
    );
    expect(getByTestId('calendar-title').textContent).toContain('Jan 13');
    fireEvent.click(getByTestId('calendar-prev-btn'));
    expect(getByTestId('calendar-title').textContent).toContain('Jan 6');
  });

  it('next_button_navigates_to_next_week_in_week_view', () => {
    const { getByTestId } = render(
      <Calendar events={[]} view="week" selectedDate={JAN15} />
    );
    fireEvent.click(getByTestId('calendar-next-btn'));
    expect(getByTestId('calendar-title').textContent).toContain('Jan 20');
  });

  it('onNavigate_called_with_date_and_view_on_prev', () => {
    const onNavigate = vi.fn();
    const { getByTestId } = render(
      <Calendar events={[]} selectedDate={JAN15} onNavigate={onNavigate} />
    );
    fireEvent.click(getByTestId('calendar-prev-btn'));
    expect(onNavigate).toHaveBeenCalledTimes(1);
    const [date, view] = onNavigate.mock.calls[0] as [Date, string];
    expect(view).toBe('month');
    expect(date.getMonth()).toBe(11); // December
    expect(date.getFullYear()).toBe(2024);
  });

  it('onNavigate_called_when_switching_view', () => {
    const onNavigate = vi.fn();
    const { getByTestId } = render(
      <Calendar events={[]} selectedDate={JAN15} onNavigate={onNavigate} />
    );
    fireEvent.click(getByTestId('calendar-view-week-btn'));
    expect(onNavigate).toHaveBeenCalledWith(expect.any(Date), 'week');
  });
});

describe('Calendar — 交互回调', () => {
  it('clicking_a_date_cell_calls_onDateSelect', () => {
    const onDateSelect = vi.fn();
    const { getByTestId } = render(
      <Calendar events={[]} selectedDate={JAN15} onDateSelect={onDateSelect} />
    );
    fireEvent.click(getByTestId('calendar-day-2025-01-20'));
    expect(onDateSelect).toHaveBeenCalledTimes(1);
    const arg = onDateSelect.mock.calls[0][0] as Date;
    expect(arg.getFullYear()).toBe(2025);
    expect(arg.getMonth()).toBe(0);
    expect(arg.getDate()).toBe(20);
  });

  it('clicking_an_event_calls_onEventClick', () => {
    const onEventClick = vi.fn();
    const evt = makeEvent({ id: 'e1' });
    const { getByTestId } = render(
      <Calendar events={[evt]} selectedDate={JAN15} onEventClick={onEventClick} />
    );
    fireEvent.click(getByTestId('calendar-event-e1'));
    expect(onEventClick).toHaveBeenCalledWith(evt);
  });
});

describe('Calendar — 事件显示', () => {
  it('events_are_color_coded_by_type', () => {
    const taskEvt = makeEvent({ id: 'task1', type: 'task', title: 'T' });
    const remEvt = makeEvent({
      id: 'rem1',
      type: 'reminder',
      title: 'R',
      start: new Date(2025, 0, 15, 14, 0),
      end: new Date(2025, 0, 15, 15, 0),
    });
    const { getByTestId } = render(
      <Calendar events={[taskEvt, remEvt]} selectedDate={JAN15} />
    );
    const taskColor = getByTestId('calendar-event-task1').getAttribute('data-color');
    const remColor = getByTestId('calendar-event-rem1').getAttribute('data-color');
    expect(taskColor).toBe('#3b82f6');
    expect(remColor).toBe('#f59e0b');
    expect(taskColor).not.toBe(remColor);
  });

  it('custom_color_overrides_type_color', () => {
    const evt = makeEvent({ id: 'c1', type: 'task', color: '#ff0000' });
    const { getByTestId } = render(<Calendar events={[evt]} selectedDate={JAN15} />);
    expect(getByTestId('calendar-event-c1').getAttribute('data-color')).toBe('#ff0000');
  });

  it('events_appear_in_correct_day_cell', () => {
    const evt = makeEvent({ id: 'e1' });
    const { getByTestId } = render(<Calendar events={[evt]} selectedDate={JAN15} />);
    const cell = getByTestId('calendar-day-2025-01-15') as HTMLElement;
    expect(cell.querySelector('[data-testid="calendar-event-e1"]')).not.toBeNull();
    // 不应出现在别的日期格里
    const other = getByTestId('calendar-day-2025-01-16') as HTMLElement;
    expect(other.querySelector('[data-testid="calendar-event-e1"]')).toBeNull();
  });

  it('more_indicator_shown_when_events_exceed_max', () => {
    const events = Array.from({ length: 4 }, (_, i) =>
      makeEvent({
        id: `e${i}`,
        start: new Date(2025, 0, 15, 9 + i, 0),
        end: new Date(2025, 0, 15, 9 + i + 1, 0),
      })
    );
    const { getByTestId, queryByTestId } = render(
      <Calendar events={events} selectedDate={JAN15} />
    );
    // 4 个事件,月格最多展示 3 个 → 折叠 1 个
    expect(getByTestId('calendar-more-2025-01-15')).toBeTruthy();
    expect(getByTestId('calendar-more-2025-01-15').textContent).toContain('1');
    // 第 4 个事件块本身不应直接渲染
    expect(queryByTestId('calendar-event-e3')).toBeNull();
  });

  it('week_view_displays_event_time_range', () => {
    const evt = makeEvent({
      id: 'e1',
      start: new Date(2025, 0, 15, 10, 30),
      end: new Date(2025, 0, 15, 12, 0),
    });
    const { getByTestId } = render(
      <Calendar events={[evt]} view="week" selectedDate={JAN15} />
    );
    const el = getByTestId('calendar-event-e1');
    expect(el.textContent).toContain('10:30');
    expect(el.textContent).toContain('12:00');
  });

  it('loop_event_type_uses_green_color', () => {
    const evt = makeEvent({ id: 'lp1', type: 'loop' });
    const { getByTestId } = render(<Calendar events={[evt]} selectedDate={JAN15} />);
    expect(getByTestId('calendar-event-lp1').getAttribute('data-color')).toBe('#10b981');
  });
});

describe('Calendar — 选中态与月外日期', () => {
  it('selected_date_cell_is_highlighted', () => {
    const { getByTestId } = render(<Calendar events={[]} selectedDate={JAN15} />);
    expect(
      getByTestId('calendar-day-2025-01-15').getAttribute('data-selected')
    ).toBe('true');
    expect(
      getByTestId('calendar-day-2025-01-20').getAttribute('data-selected')
    ).toBe('false');
  });

  it('outside_month_days_are_dimmed', () => {
    // Jan 2025 网格从 2024-12-30(周一)开始
    const { getByTestId } = render(<Calendar events={[]} selectedDate={JAN15} />);
    const outside = getByTestId('calendar-day-2024-12-30') as HTMLElement;
    expect(outside.getAttribute('data-in-month')).toBe('false');
    expect(outside.style.opacity).toBe('0.4');
    const inside = getByTestId('calendar-day-2025-01-15') as HTMLElement;
    expect(inside.getAttribute('data-in-month')).toBe('true');
    expect(inside.style.opacity).toBe('1');
  });

  it('clicking_a_date_updates_selected_highlight', () => {
    const { getByTestId } = render(<Calendar events={[]} selectedDate={JAN15} />);
    expect(
      getByTestId('calendar-day-2025-01-15').getAttribute('data-selected')
    ).toBe('true');
    fireEvent.click(getByTestId('calendar-day-2025-01-20'));
    expect(
      getByTestId('calendar-day-2025-01-20').getAttribute('data-selected')
    ).toBe('true');
    expect(
      getByTestId('calendar-day-2025-01-15').getAttribute('data-selected')
    ).toBe('false');
  });
});

describe('Calendar — i18n', () => {
  it('zh_cn_locale_renders_chinese_strings', () => {
    setLocale('zh-CN');
    const { getByTestId } = render(<Calendar events={[]} selectedDate={JAN15} />);
    expect(getByTestId('calendar-today-btn').textContent).toBe('今天');
    expect(getByTestId('calendar-view-month-btn').textContent).toBe('月');
    expect(getByTestId('calendar-view-week-btn').textContent).toBe('周');
    expect(getByTestId('calendar-weekday-0').textContent).toContain('一');
    expect(getByTestId('calendar-title').textContent).toContain('2025年1月');
  });

  it('en_us_locale_renders_english_month_title', () => {
    const { getByTestId } = render(<Calendar events={[]} selectedDate={JAN15} />);
    expect(getByTestId('calendar-title').textContent).toContain('January 2025');
  });
});
