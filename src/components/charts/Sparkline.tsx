/** T-E-A-07: 纯 SVG 折线图组件(零依赖,与现有 MetricCard CSS 风格一致)。 */

export interface SparklineProps {
  data: number[];
  width?: number;
  height?: number;
  color?: string;
  threshold?: number;
  thresholdColor?: string;
}

export function Sparkline({
  data,
  width = 300,
  height = 60,
  color = '#39d98a',
  threshold,
  thresholdColor = '#ef4444',
}: SparklineProps) {
  if (data.length === 0) {
    return <svg width={width} height={height} />;
  }

  const max = Math.max(...data, threshold ?? 0, 0.01);
  const min = Math.min(...data, 0);
  const range = max - min || 1;
  const stepX = data.length > 1 ? width / (data.length - 1) : width;

  const points = data.map((v, i) => {
    const x = i * stepX;
    const y = height - ((v - min) / range) * (height - 4) - 2;
    return [x, y] as const;
  });

  const linePath = points.map(([x, y]) => `${x},${y}`).join(' ');
  const fillPath = `0,${height} ${linePath} ${width},${height}`;

  return (
    <svg width={width} height={height} style={{ overflow: 'visible' }}>
      <polygon points={fillPath} fill={color} fillOpacity={0.1} />
      <polyline
        points={linePath}
        fill="none"
        stroke={color}
        strokeWidth={1.5}
        strokeLinejoin="round"
        strokeLinecap="round"
      />
      {threshold !== undefined && threshold > 0 && (
        <>
          <line
            x1={0}
            y1={height - ((threshold - min) / range) * (height - 4) - 2}
            x2={width}
            y2={height - ((threshold - min) / range) * (height - 4) - 2}
            stroke={thresholdColor}
            strokeWidth={1}
            strokeDasharray="4,3"
          />
          <text
            x={width - 4}
            y={height - ((threshold - min) / range) * (height - 4) - 6}
            fill={thresholdColor}
            fontSize={9}
            textAnchor="end"
          >
            budget
          </text>
        </>
      )}
    </svg>
  );
}
