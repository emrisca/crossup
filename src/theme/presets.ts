/**
 * Clickable presets: one hue and one chroma each.
 *
 * Mode is deliberately not part of a preset, the dark/light switch is its own
 * control, so picking a palette never silently flips the app.
 */
export type Preset = {
  id: string;
  name: string;
  hue: number;
  chroma: number;
};

export const PRESETS: Preset[] = [
  { id: "indigo", name: "Indigo", hue: 264, chroma: 0.05 },
  { id: "graphite", name: "Graphite", hue: 264, chroma: 0 },
  { id: "slate", name: "Slate", hue: 250, chroma: 0.02 },
  { id: "tide", name: "Tide", hue: 220, chroma: 0.09 },
  { id: "sage", name: "Sage", hue: 150, chroma: 0.04 },
  { id: "moss", name: "Moss", hue: 130, chroma: 0.08 },
  { id: "ember", name: "Ember", hue: 40, chroma: 0.1 },
  { id: "clay", name: "Clay", hue: 24, chroma: 0.06 },
  { id: "rose", name: "Rose", hue: 340, chroma: 0.08 },
  { id: "violet", name: "Violet", hue: 300, chroma: 0.12 },
  { id: "vivid", name: "Vivid", hue: 285, chroma: 0.2 },
];
