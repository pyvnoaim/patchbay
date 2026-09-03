# Changelog

Written for someone about to install, not for someone reading the diff: what
changed for them, in plain sentences. Newest first.

This file is the source. `npm run bump` stamps `## Unreleased` with the version
and the date, the release workflow lifts that section into the GitHub release,
and from there the app shows it before it installs anything and the site reads
it back. Nothing is retyped anywhere.

## Unreleased

- A device's forwards can be held open on their own, without a session, from the
  Forwards section of its pane.
- A space has a page of its own now: which file it is, and what its sync is doing.
- The window opens where you left it, and the sidebar keeps the width you drag it to.
- Terminal sessions use patchbay's colours, not the raw VT palette. Links are clickable.
- Clicking a device you already have open goes to that tab.
- patchbay updates itself. It asks once when the window opens, downloads only
  when you say so, and never restarts you: the new version is put in place and
  the button becomes `Restart`, which is a separate click because it takes your
  live sessions with it. Settings ▸ Updates has the switch and a check you can
  run yourself; the macOS menu has one too.
- The disk image is an installer now, with the app and Applications where you
  expect them, and it ejects itself once you've dragged the app out.
- Everything downloaded is verified against a key built into the app before a
  single file is replaced.
