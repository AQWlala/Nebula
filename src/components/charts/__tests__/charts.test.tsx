import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/preact';
import { BarChart } from '../BarChart';
import { Sparkline } from '../Sparkline';

describe('BarChart', () => {
  it('renders empty svg when data is empty', () => {
    const { container } = render(<BarChart data={[]} width={300} height={120} />);
    const svg = container.querySelector('svg');
    expect(svg).not.toBeNull();
    expect(svg?.getAttribute('width')).toBe('300');
    expect(svg?.querySelectorAll('rect')).toHaveLength(0);
  });

  it('renders bars for each data point', () => {
    const data = [
      { label: 'A', value: 10 },
      { label: 'B', value: 20 },
      { label: 'C', value: 5 },
    ];
    const { container } = render(<BarChart data={data} />);
    expect(container.querySelectorAll('rect')).toHaveLength(3);
    expect(container.querySelectorAll('text')).toHaveLength(6); // 2 text per bar (value + label)
  });

  it('uses custom color', () => {
    const { container } = render(
      <BarChart data={[{ label: 'X', value: 1 }]} color="#ff0000" />
    );
    const rect = container.querySelector('rect');
    expect(rect?.getAttribute('fill')).toBe('#ff0000');
  });

  it('uses custom valueFormatter', () => {
    const { container } = render(
      <BarChart
        data={[{ label: 'X', value: 0.5 }]}
        valueFormatter={(v) => `${Math.round(v * 100)}%`}
      />
    );
    // 第一个 text 是 value,第二个是 label
    const texts = container.querySelectorAll('text');
    expect(texts[0]?.textContent).toBe('50%');
  });

  it('truncates long labels', () => {
    const { container } = render(
      <BarChart data={[{ label: 'very-long-label-name', value: 1 }]} />
    );
    const texts = container.querySelectorAll('text');
    const labelText = texts[texts.length - 1]?.textContent;
    expect(labelText).toBe('very-long-..');
  });

  it('handles single data point', () => {
    const { container } = render(
      <BarChart data={[{ label: 'only', value: 42 }]} />
    );
    expect(container.querySelectorAll('rect')).toHaveLength(1);
  });
});

describe('Sparkline', () => {
  it('renders empty svg when data is empty', () => {
    const { container } = render(<Sparkline data={[]} width={300} height={60} />);
    const svg = container.querySelector('svg');
    expect(svg).not.toBeNull();
    expect(svg?.querySelector('polygon')).toBeNull();
    expect(svg?.querySelector('polyline')).toBeNull();
  });

  it('renders polyline and polygon for data', () => {
    const { container } = render(<Sparkline data={[1, 2, 3, 2, 1]} />);
    expect(container.querySelector('polygon')).not.toBeNull();
    expect(container.querySelector('polyline')).not.toBeNull();
  });

  it('renders threshold line when provided', () => {
    const { container } = render(
      <Sparkline data={[1, 2, 3]} threshold={2.5} thresholdColor="#ff0000" />
    );
    const line = container.querySelector('line');
    expect(line).not.toBeNull();
    expect(line?.getAttribute('stroke')).toBe('#ff0000');
    // threshold 文本
    const text = container.querySelector('text');
    expect(text?.textContent).toBe('budget');
  });

  it('omits threshold line when threshold is 0', () => {
    const { container } = render(<Sparkline data={[1, 2, 3]} threshold={0} />);
    expect(container.querySelector('line')).toBeNull();
  });

  it('omits threshold line when threshold is undefined', () => {
    const { container } = render(<Sparkline data={[1, 2, 3]} />);
    expect(container.querySelector('line')).toBeNull();
  });

  it('handles single data point', () => {
    const { container } = render(<Sparkline data={[5]} />);
    // 单点也应渲染 polyline(虽然退化)
    expect(container.querySelector('polyline')).not.toBeNull();
  });

  it('uses custom color for stroke and fill', () => {
    const { container } = render(
      <Sparkline data={[1, 2, 3]} color="#abcdef" />
    );
    const polyline = container.querySelector('polyline');
    const polygon = container.querySelector('polygon');
    expect(polyline?.getAttribute('stroke')).toBe('#abcdef');
    expect(polygon?.getAttribute('fill')).toBe('#abcdef');
  });
});
