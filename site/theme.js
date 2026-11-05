// Runs from <head> before the body paints: the theme is an attribute and the first frame needs
// it, or a pinned light page flashes dark. Nothing stored means the system decides, in CSS.
try {
  var pinned = localStorage.getItem("theme");
  if (pinned === "light" || pinned === "dark")
    document.documentElement.setAttribute("data-theme", pinned);
} catch (e) {}
