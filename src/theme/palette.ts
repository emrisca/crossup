/**
 * UI-Colors, reimplemented.
 *
 * One hue (20–340°) and one chroma (0–0.2) derive fourteen named colours in
 * OKLCH: three backgrounds, two ink steps, three edges, two actions and four
 * alerts. This is the arithmetic of
 * [whosajid/UI-Colors](https://github.com/whosajid/UI-Colors) — same lightness
 * ladder per mode, same chroma floors so actions stay visible at zero
 * saturation, secondary action half a turn around the wheel.
 *
 * Interior's materials are then mapped onto those fourteen tokens, and a tinted
 * neutral ramp is derived for the `stone` utilities the components use.
 */

export type Mode = "dark" | "light";

export type Palette = {
  hue: number;
  chroma: number;
  mode: Mode;
};

export type Oklch = {
  l: number;
  c: number;
  h: number;
};

export const HUE_MIN = 20;
export const HUE_MAX = 340;
export const CHROMA_MIN = 0;
export const CHROMA_MAX = 0.2;

/** Upstream's starting sliders: an indigo-leaning palette with soft greys. */
export const DEFAULT_PALETTE: Palette = { hue: 264, chroma: 0.05, mode: "dark" };

export type Tokens = {
  bgDark: Oklch;
  bg: Oklch;
  bgLight: Oklch;
  text: Oklch;
  textMuted: Oklch;
  highlight: Oklch;
  border: Oklch;
  borderMuted: Oklch;
  primary: Oklch;
  secondary: Oklch;
  danger: Oklch;
  warning: Oklch;
  success: Oklch;
  info: Oklch;
};

const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value));

/** Both sliders are clamped to their track, so a stray value cannot escape. */
export function clampPalette(palette: Palette): Palette {
  return {
    hue: clamp(palette.hue, HUE_MIN, HUE_MAX),
    chroma: clamp(palette.chroma, CHROMA_MIN, CHROMA_MAX),
    mode: palette.mode,
  };
}

/** `0.15000` → `0.15`, `1.0` → `1`, `264.0` → `264`. */
const trim = (value: number): string => {
  const text = value.toFixed(3).replace(/0+$/, "").replace(/\.$/, "");
  return text === "" || text === "-0" ? "0" : text;
};

export const oklchCss = (color: Oklch): string =>
  `oklch(${trim(color.l)} ${trim(color.c)} ${trim(color.h)})`;

const midpoint = (a: Oklch, b: Oklch): Oklch => ({
  l: (a.l + b.l) / 2,
  c: (a.c + b.c) / 2,
  h: a.h,
});

/** Derive the fourteen tokens, exactly as upstream's `getResolvedCssVars` does. */
export function tokensOf(palette: Palette): Tokens {
  const { hue, chroma, mode } = clampPalette(palette);
  const secondaryHue = (hue + 180) % 360;

  const background = chroma * 0.5;
  const text = Math.min(chroma, 0.1);
  const action = Math.max(chroma, 0.1);
  const alert = Math.max(chroma, 0.05);

  const at = (l: number, c: number, h: number): Oklch => ({ l, c, h });

  if (mode === "dark") {
    return {
      bgDark: at(0.1, background, hue),
      bg: at(0.15, background, hue),
      bgLight: at(0.2, background, hue),
      text: at(0.96, text, hue),
      textMuted: at(0.76, text, hue),
      highlight: at(0.5, chroma, hue),
      border: at(0.4, chroma, hue),
      borderMuted: at(0.3, chroma, hue),
      primary: at(0.76, action, hue),
      secondary: at(0.76, action, secondaryHue),
      danger: at(0.7, alert, 30),
      warning: at(0.7, alert, 100),
      success: at(0.7, alert, 160),
      info: at(0.7, alert, 260),
    };
  }

  return {
    bgDark: at(0.92, background, hue),
    bg: at(0.96, background, hue),
    bgLight: at(1.0, background, hue),
    text: at(0.15, chroma, hue),
    textMuted: at(0.4, chroma, hue),
    highlight: at(1.0, chroma, hue),
    border: at(0.6, chroma, hue),
    borderMuted: at(0.7, chroma, hue),
    primary: at(0.4, action, hue),
    secondary: at(0.4, action, secondaryHue),
    danger: at(0.5, alert, 30),
    warning: at(0.5, alert, 100),
    success: at(0.5, alert, 160),
    info: at(0.5, alert, 260),
  };
}

/**
 * The palette mapped onto Interior's material tokens.
 *
 * The three backgrounds keep their order — frame, recessed well, lifted panel —
 * so the depth the components are built around survives the recolour. Shadows
 * are left alone: depth is what makes the materials read as materials.
 */
export function interiorVariables(tokens: Tokens): Record<string, string> {
  const ink2 = midpoint(tokens.text, tokens.textMuted);
  const sub = midpoint(tokens.bgDark, tokens.bg);
  return {
    "--bezel": oklchCss(tokens.bgDark),
    "--panel": oklchCss(tokens.bg),
    "--well": oklchCss(tokens.bgLight),
    "--sub": oklchCss(sub),
    "--ink": oklchCss(tokens.text),
    "--ink-2": oklchCss(ink2),
    "--ink-3": oklchCss(tokens.textMuted),
    "--accent": oklchCss(tokens.primary),
    "--accent-soft": `color-mix(in oklab, ${oklchCss(tokens.primary)} 14%, transparent)`,
    "--moss": oklchCss(tokens.success),
    "--flag": oklchCss(tokens.danger),
    "--hairline": `color-mix(in oklab, ${oklchCss(tokens.borderMuted)} 55%, transparent)`,
    "--hairline-strong": `color-mix(in oklab, ${oklchCss(tokens.border)} 72%, transparent)`,
  };
}

/**
 * Tailwind's `stone` ladder, retinted.
 *
 * The components express most of their surfaces and muted ink with `stone-*`
 * utilities, and Tailwind v4 resolves those through `--color-stone-*`. Overriding
 * the eleven steps at runtime therefore recolours ~640 utility usages at once,
 * while the lightness ladder — and so every contrast decision baked into the
 * components — stays where it was. Chroma is scaled well below the slider's
 * value: these are greys that lean, not colours.
 */
export function stoneRamp(palette: Palette): Record<string, string> {
  const { hue, chroma } = clampPalette(palette);
  const tint = chroma * 0.15;
  const ladder: [string, number][] = [
    ["50", 0.985],
    ["100", 0.97],
    ["200", 0.923],
    ["300", 0.869],
    ["400", 0.709],
    ["500", 0.553],
    ["600", 0.444],
    ["700", 0.374],
    ["800", 0.268],
    ["900", 0.216],
    ["950", 0.147],
  ];

  const ramp: Record<string, string> = {};
  for (const [step, lightness] of ladder) {
    ramp[`--color-stone-${step}`] = oklchCss({ l: lightness, c: tint, h: hue });
  }
  return ramp;
}
