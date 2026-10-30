export type Panel = 'files' | 'agent' | 'results';
export type Pane = Panel | 'editor';
export type LayoutPreferences = {
  sizes: Record<Panel, number>;
  collapsed: Record<Panel, boolean>;
  priority: 'files' | 'agent';
};
export const layoutLimits = {
  rail: 40, divider: 5, mobile: 769, editorWidth: 420, editorHeight: 220,
  files: { min: 180, max: 380, initial: 224 },
  agent: { min: 300, max: 600, initial: 344 },
  results: { min: 140, max: 600, initial: 240 },
} as const;
export const clamp = (value: number, min: number, max: number) => Math.round(Math.max(min, Math.min(max, value)));

export function readLayout(value: unknown): LayoutPreferences {
  const record = value && typeof value === 'object' ? value as Partial<LayoutPreferences> : {};
  const sizes = {} as Record<Panel, number>, collapsed = {} as Record<Panel, boolean>;
  for (const panel of ['files', 'agent', 'results'] as const) {
    const size = record.sizes?.[panel], limit = layoutLimits[panel];
    sizes[panel] = typeof size === 'number' && Number.isFinite(size) ? clamp(size, limit.min, limit.max) : limit.initial;
    collapsed[panel] = typeof record.collapsed?.[panel] === 'boolean' ? record.collapsed[panel] : panel !== 'files';
  }
  return { sizes, collapsed, priority: record.priority === 'agent' ? 'agent' : 'files' };
}

/** Preferences survive viewport pressure. Only explicit user actions change them. */
export function resolveLayout(preferences: LayoutPreferences, width: number, height: number) {
  const { rail, divider, editorWidth, editorHeight } = layoutLimits;
  const collapsed = { ...preferences.collapsed };
  const sides = ['files', 'agent'] as const;
  const sideBudget = Math.max(2 * rail, Math.floor(width) - editorWidth - 2 * divider);
  const minimum = () => sides.reduce((sum, panel) => sum + (collapsed[panel] ? rail : layoutLimits[panel].min), 0);
  for (const panel of [preferences.priority === 'files' ? 'agent' : 'files', preferences.priority] as const) {
    if (minimum() > sideBudget) collapsed[panel] = true;
  }
  const sizes = { ...preferences.sizes };
  for (const panel of sides) if (collapsed[panel]) sizes[panel] = rail;
  const excess = Math.max(0, sizes.files + sizes.agent - sideBudget);
  const slack = sides.map(panel => collapsed[panel] ? 0 : sizes[panel] - layoutLimits[panel].min);
  if (excess > 0 && slack[0] + slack[1] > 0) {
    const filesReduction = Math.min(slack[0], Math.ceil(excess * slack[0] / (slack[0] + slack[1])));
    sizes.files -= filesReduction;
    sizes.agent -= Math.min(slack[1], excess - filesReduction);
  }
  const resultsMax = Math.min(layoutLimits.results.max, Math.floor(height) - editorHeight - divider);
  if (resultsMax < layoutLimits.results.min) collapsed.results = true;
  sizes.results = collapsed.results ? rail : clamp(sizes.results, layoutLimits.results.min, resultsMax);
  const maximum = {
    files: Math.min(layoutLimits.files.max, sideBudget - sizes.agent),
    agent: Math.min(layoutLimits.agent.max, sideBudget - sizes.files),
    results: resultsMax,
  };
  return { mobile: width <= layoutLimits.mobile, collapsed, sizes, maximum };
}
