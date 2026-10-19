export type Theme = {
  id: string;
  name: string;
  /// Preview swatches: [background, accent, accent2/panel]
  swatch: [string, string, string];
};

export const THEMES: Theme[] = [
  { id: "default", name: "Kakiri", swatch: ["#0e0b16", "#8b5cf6", "#b79bff"] },
  { id: "dark", name: "Dark", swatch: ["#000000", "#1d9bf0", "#2f3336"] },
];
