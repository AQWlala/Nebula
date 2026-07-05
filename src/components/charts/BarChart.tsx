/** T-E-A-07: 纯 SVG 柱状图组件(零依赖)。 */

export interface BarChartProps {
  data: { label: string; value: number }[];
  width?: number;
  height?: number;
  color?: string;
  valueFormatter?: (v: number) => string;
}

export function BarChart({
  data,
  width = 300,
  height = 120,
  color = '#3b82f6',
  valueFormatter = (v) => v.toFixed(2),
}: BarChartProps) {
  if (data.length === 0) {
    return <svg width={width} height={height} />;
  }

  const maxVal = Math.max(...data.map((d) => d.value), 0.01);
  const barWidth = width / data.length;
  const labelHeight = 20;
  const barAreaHeight = height - labelHeight - 16;

  return (
    <svg width={width} height={height}>
      {data.map((d, i) => {
        const barHeight = (d.value / maxVal) * barAreaHeight;
        const x = i * barWidth + 2;
        const y = barAreaHeight - barHeight + 4;
        const truncatedLabel =
          d.label.length > 12 ? d.label.slice(0, 10) + '..' : d.label;
        return (
          <g key={d.label}>
            <rect
              x={x}
              y={y}
              width={barWidth - 4}
              height={barHeight}
              fill={color}
              rx={2}
              fillOpacity={0.8}
            />
            <text
              x={x + (barWidth - 4) / 2}
              y={y - 4}
              fill="var(--text-secondary, #888)"
              fontSize={9}
              textAnchor="middle"
            >
              {valueFormatter(d.value)}
            </text>
            <text
              x={x + (barWidth - 4) / 2}
              y={height - 6}
              fill="var(--text-secondary, #888)"
              fontSize={9}
              textAnchor="middle"
            >
              {truncatedLabel}
            </text>
          </g>
        );
      })}
    </svg>
  );
}
