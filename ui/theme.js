// Runs from <head>, before the body paints: the theme is an attribute, and the first
// frame needs it. `localStorage` remembers last time's answer until load() reads the
// real one from [settings].
{
  let saved = null;
  try {
    saved = localStorage.theme;
  } catch {
    /* no storage: the system answers */
  }
  document.documentElement.dataset.theme =
    saved === "light" || saved === "dark"
      ? saved
      : matchMedia("(prefers-color-scheme: light)").matches
        ? "light"
        : "dark";
}
