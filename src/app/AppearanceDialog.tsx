import type { CSSProperties } from "react";
import { Modal } from "@/components/interior/modal";
import { PressDepth } from "@/components/interior/press-depth";
import { SegmentedControl } from "@/components/interior/segmented-control";
import { useTheme } from "@/theme/ThemeProvider";
import { PRESETS } from "@/theme/presets";
import {
  CHROMA_MAX,
  CHROMA_MIN,
  HUE_MAX,
  HUE_MIN,
  oklchCss,
  tokensOf,
  type Mode,
} from "@/theme/palette";

/** The whole colour wheel, fixed, so the hue track does not move while dragging. */
const HUE_TRACK =
  "linear-gradient(90deg, oklch(0.62 0.16 20), oklch(0.62 0.16 100), " +
  "oklch(0.62 0.16 180), oklch(0.62 0.16 260), oklch(0.62 0.16 340))";

type AppearanceDialogProps = {
  open: boolean;
  onClose: () => void;
};

export function AppearanceDialog({ open, onClose }: AppearanceDialogProps) {
  const { palette, tokens, setHue, setChroma, setMode, reset } = useTheme();
  const onAccent = palette.mode === "dark" ? "#0b0b0b" : "#ffffff";

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="Appearance"
      description="One hue and one saturation derive the whole palette. Crossup paints every surface in the window with it, so this dialog is the preview."
      maxWidth={600}
      maxHeight="min(84vh, 720px)"
      footer={
        <div style={{ display: "flex", alignItems: "center", gap: 10, width: "100%" }}>
          <button type="button" className="appearance__reset" onClick={reset}>
            Reset
          </button>
          <span className="studio__spacer" />
          <PressDepth depth={3} tilt={5} onClick={onClose}>
            Done
          </PressDepth>
        </div>
      }
    >
      <div className="appearance">
        <section>
          <h3 className="appearance__label">Mode</h3>
          <SegmentedControl
            label="Colour mode"
            options={[
              { value: "dark", label: "Dark" },
              { value: "light", label: "Light" },
            ]}
            value={palette.mode}
            onValueChange={(value) => setMode(value as Mode)}
          />
        </section>

        <section>
          <h3 className="appearance__label">Presets</h3>
          <div className="appearance__presets">
            {PRESETS.map((preset) => {
              const presetTokens = tokensOf({
                hue: preset.hue,
                chroma: preset.chroma,
                mode: palette.mode,
              });
              const active =
                Math.abs(palette.hue - preset.hue) < 0.5 &&
                Math.abs(palette.chroma - preset.chroma) < 0.005;
              return (
                <button
                  key={preset.id}
                  type="button"
                  className="appearance__preset"
                  aria-pressed={active}
                  onClick={() => {
                    setHue(preset.hue);
                    setChroma(preset.chroma);
                  }}
                >
                  <span
                    className="appearance__preset-swatch"
                    aria-hidden="true"
                    style={{
                      background: `linear-gradient(135deg, ${oklchCss(presetTokens.primary)}, ${oklchCss(presetTokens.secondary)})`,
                    }}
                  />
                  {preset.name}
                </button>
              );
            })}
          </div>
        </section>

        <section>
          <h3 className="appearance__label">Custom</h3>
          <div className="appearance__slider">
            <div className="appearance__slider-head">
              <span id="appearance-chroma">Saturation</span>
              <span className="appearance__slider-value">
                {palette.chroma.toFixed(2)}
              </span>
            </div>
            <input
              className="appearance__range"
              type="range"
              min={CHROMA_MIN}
              max={CHROMA_MAX}
              step={0.01}
              value={palette.chroma}
              aria-labelledby="appearance-chroma"
              onChange={(event) => setChroma(Number(event.target.value))}
              style={
                {
                  "--track": `linear-gradient(90deg, ${oklchCss({ l: 0.62, c: CHROMA_MIN, h: palette.hue })}, ${oklchCss({ l: 0.62, c: CHROMA_MAX, h: palette.hue })})`,
                } as CSSProperties
              }
            />
          </div>
          <div className="appearance__slider">
            <div className="appearance__slider-head">
              <span id="appearance-hue">Hue</span>
              <span className="appearance__slider-value">
                {Math.round(palette.hue)}°
              </span>
            </div>
            <input
              className="appearance__range"
              type="range"
              min={HUE_MIN}
              max={HUE_MAX}
              step={1}
              value={palette.hue}
              aria-labelledby="appearance-hue"
              onChange={(event) => setHue(Number(event.target.value))}
              style={{ "--track": HUE_TRACK } as CSSProperties}
            />
          </div>
        </section>

        <section>
          <h3 className="appearance__label">Preview</h3>
          <div className="appearance__preview">
            <span className="appearance__preview-eyebrow">Release notes</span>
            <p className="appearance__preview-title">Version 0.1.0</p>
            <p className="appearance__preview-line">
              Muted ink is one step dimmer than the heading, and borders stay light.
            </p>
            <div className="appearance__preview-row">
              <span
                className="appearance__preview-btn"
                style={{ background: oklchCss(tokens.primary), color: onAccent }}
              >
                Primary
              </span>
              <span
                className="appearance__preview-btn"
                style={{ background: oklchCss(tokens.secondary), color: onAccent }}
              >
                Secondary
              </span>
              {(
                [
                  ["Success", tokens.success],
                  ["Warning", tokens.warning],
                  ["Danger", tokens.danger],
                ] as const
              ).map(([label, color]) => (
                <span
                  key={label}
                  className="appearance__preview-tag"
                  style={{
                    color: oklchCss(color),
                    background: `color-mix(in oklab, ${oklchCss(color)} 14%, transparent)`,
                  }}
                >
                  {label}
                </span>
              ))}
            </div>
          </div>
        </section>
      </div>
    </Modal>
  );
}
