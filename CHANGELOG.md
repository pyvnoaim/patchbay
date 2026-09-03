# Changelog

Written for someone about to install, not for someone reading the diff: what
changed for them, in plain sentences. Newest first.

This file is the source. `npm run bump` stamps `## Unreleased` with the version
and the date, the release workflow lifts that section into the GitHub release,
and from there the app shows it before it installs anything and the site reads
it back. Nothing is retyped anywhere.

## Unreleased

- Terminal sessions use patchbay's own colours instead of the raw VT palette, so a
  failing unit is the same red as a device that's down. Links in output are
  clickable, and they open in your browser.
- Clicking a device you already have open takes you to that tab instead of opening
  a second one.
- patchbay updates itself. It asks once when the window opens, downloads only
  when you say so, and never restarts you: the new version is put in place and
  the button becomes `Restart`, which is a separate click because it takes your
  live sessions with it. Settings ▸ Updates has the switch and a check you can
  run yourself; the macOS menu has one too.
- The disk image is an installer now, with the app and Applications where you
  expect them, and it ejects itself once you've dragged the app out.
- Everything downloaded is verified against a key built into the app before a
  single file is replaced.
