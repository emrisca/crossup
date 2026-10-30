import {
  createContext,
  useCallback,
  useContext,
  useLayoutEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import {
  DEFAULT_PALETTE,
  clampPalette,
  interiorVariables,
  stoneRamp,
  tokensOf,
  type Mode,
  type Palette,
  type Tokens,
} from "./palette";

const STORAGE_KEY = "crossup.palette";

function readStored(): Palette {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_PALETTE;
    const parsed = JSON.parse(raw) as Partial<Palette>;
    if (typeof parsed.hue !== "number" || typeof parsed.chroma !== "number") {
      return DEFAULT_PALETTE;
    }
    return clampPalette({
      hue: parsed.hue,
      chroma: parsed.chroma,
      mode: parsed.mode === "light" ? "light" : "dark",
    });
  } catch {
    return DEFAULT_PALETTE;
  }
}

/**
 * Write the palette onto the document root.
 *
 * The root, not a wrapper: several components render through `createPortal`
 * (modal, drawer, lightbox, command palette, context menu), so anything applied
 * to a subtree would leave those dialogs on the default palette.
 */
export function applyPalette(palette: Palette): void {
  const root = document.documentElement;
  const variables = {
    ...interiorVariables(tokensOf(palette)),
    ...stoneRamp(palette),
  };
  for (const [name, value] of Object.entries(variables)) {
    root.style.setProperty(name, value);
  }
  root.classList.toggle("dark", palette.mode === "dark");
  root.dataset.theme = palette.mode;
  root.style.colorScheme = palette.mode;
}

type ThemeContextValue = {
  palette: Palette;
  tokens: Tokens;
  setPalette: (palette: Palette) => void;
  setHue: (hue: number) => void;
  setChroma: (chroma: number) => void;
  setMode: (mode: Mode) => void;
  toggleMode: () => void;
  reset: () => void;
};

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [palette, setPaletteState] = useState<Palette>(readStored);

  useLayoutEffect(() => {
    applyPalette(palette);
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(palette));
    } catch {
      // A private webview with storage disabled still gets the palette.
    }
  }, [palette]);

  const setPalette = useCallback((next: Palette) => {
    setPaletteState(clampPalette(next));
  }, []);

  const value = useMemo<ThemeContextValue>(
    () => ({
      palette,
      tokens: tokensOf(palette),
      setPalette,
      setHue: (hue) => setPaletteState((current) => clampPalette({ ...current, hue })),
      setChroma: (chroma) =>
        setPaletteState((current) => clampPalette({ ...current, chroma })),
      setMode: (mode) => setPaletteState((current) => ({ ...current, mode })),
      toggleMode: () =>
        setPaletteState((current) => ({
          ...current,
          mode: current.mode === "dark" ? "light" : "dark",
        })),
      reset: () => setPaletteState(DEFAULT_PALETTE),
    }),
    [palette, setPalette],
  );

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const value = useContext(ThemeContext);
  if (!value) throw new Error("useTheme must be used inside <ThemeProvider>");
  return value;
}
