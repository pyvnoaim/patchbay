// Loaded in <head>, before the body paints. The theme is an attribute rather than a
// media query, so something has to set it before the first frame - and [settings]
// lives in a file we cannot read from here. `localStorage` is this window's memory of
// the answer it settled on last time; load() replaces it the moment it has read the
// real one.
{
  let saved = null;
  try { saved = localStorage.theme; } catch { /* no storage: the system answers */ }
  document.documentElement.dataset.theme = saved === "light" || saved === "dark" ? saved
    : matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
}
