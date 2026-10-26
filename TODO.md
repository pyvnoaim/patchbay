# TODO

What's next, then ideas. A line leaves this file when it ships (the changelog says it
then) or when it's decided against (say why in the commit).

## Now: try the unreleased work in the real app

Built and unit-tested, never run in the window. `npm run dev`, then tick these off before
the next push.

- [ ] **VNC tab** on a Linux desktop, Proxmox/QEMU console or KVM: typing (with Shift and
      ⌥), the remote cursor's shape, clipboard both ways (copy there, paste here; copy
      here, click in, paste there).
- [ ] **VNC password**: a server that wants one asks; Remember works; a wrong saved one is
      asked again still ticked. A Mac with only Apple's sign-in opens in Screen Sharing.
- [ ] **RDP regression** (its canvas code moved): typing, ⌘C/⌘V, resizing the window,
      the 2008 R2 handoff, a changed certificate.
- [ ] **RDP pointer**: the I-beam over text and resize arrows on a window edge, at the
      right size on a Retina screen.
- [ ] **RDP sound**: a video or a system sound plays, in sync. Does it start at once or
      only after the first click in the desktop?
- [ ] **Settings**: a switch takes effect at once; theme, font size and the shared list
      save on leaving the field; closing with × saves a half-typed field; an invalid
      snippet keeps the sheet open, closing twice discards it.
- [ ] **The 0.1.63 shell**, in light and dark: the device page, sidebar, quick connect,
      the device form (More, Advanced, the ssh preview) and Settings at a narrow and a
      wide window.
- [ ] **Background tabs**: output in another tab lights its dot; a drop and a bell there
      raise one notice and one Dock bounce, not one per line.
- [ ] **Haptics** on a Force Touch trackpad: a device dragged onto a folder ticks; the
      sidebar edge settles on its default width with a tick.
- [ ] **Telnet**: a device with `telnet = 23`, without `telnet` installed (the curl
      fallback): login prompt, typing, echo, ^C. Once behind a jump host.
- [ ] **Serial**: a console cable at `/dev/cu.usbserial-…`, 9600 and 115200.
- [ ] **Split panes**: ⌘D beside a shell, an RDP tab and a web tab; drag and double-click
      the divider; ⌘[ / ⌘] through panes; ⌘W one pane; "Stack the panes" and
      "Separate into tabs" from the tab's menu; a second shell of the same device.
- [ ] **VoiceOver** (⌘F5): the tree reads names and status, tabs read as tabs, icon
      buttons have names, the menu key opens a device's menu.
- [ ] **Licence pane**: with one vault it says free; with a shared list it shows no
      licence needed yet; a fresh `npm run licence` paste is accepted and removable.

## Next: the paid version

The code is in: `licence.rs`, Settings ▸ Licence, and `npm run licence "Acme GmbH"
2027-10-20` to sign one with `~/.tauri/patchbay-licence.key`. Free for one person; a team
on a shared list needs a company licence, €249 a year, checked offline. When one ends the
list keeps opening for a week under a "licence ended" line, then stops; your own devices
always open. None of it asks for anything while `licence::ON_SALE` is false.

Left, in this order:

1. Open a Lemon Squeezy store with the company licence as a yearly product. Licence
   keys off: the licence is ours, signed by `npm run licence` and sent by hand while
   orders are few.
2. Pick the domain. `patchbay.dev` is a placeholder in `site/index.html`.
3. Add the checkout link to the site and the README (the €249 card's Buy button, and
   drop its "on sale soon"), and a Buy link in Settings ▸ Licence.
4. Have someone who knows licences read `LICENSE`. It is plain words, not a lawyer's.
5. Sign a real licence, paste it on a shared list with `ON_SALE` on in a dev build, and
   check the sidebar line goes; one that has ended closes the list after the week.
6. The launch is 1.0.0 (`npm run bump 1.0.0`), never a reset to 0.1.0: the updater only
   moves up. Turn `ON_SALE` on in the same commit. Make `LICENSE`'s last line "Versions
   before 1.0.0", but only if nothing after 0.1.58 has shipped under Perimeter.
7. Once 1.0.0 is published, delete the 0.1.x GitHub releases (`gh release delete <tag>
   --cleanup-tag`). Not before: installed copies update through the latest release's
   `latest.json`, so 1.0.0 has to be live first. It stops new downloads only; copies
   people have and the source in git history stay under Perimeter.

Later, if orders aren't few: a Lemon Squeezy webhook into a function that runs the same
signing, so a licence arrives with the receipt.

## Next: IronRDP back from the fork

The graphics pipeline needs IronRDP's main branch, pinned in `src-tauri/Cargo.toml` to
`pyvnoaim/IronRDP`: that main plus two speedups sent upstream as Devolutions/IronRDP#2216
and #2217. Once both merge, point the pin at Devolutions' repo. The pipeline is on
crates.io since 2026-10-23, but `ironrdp-egfx-native` and a new `ironrdp` aren't; once
they are, go back to crates.io.

## Ideas

Rough, unranked. Most are a `ponytail:` ceiling someone hasn't hit yet.

- **Shared host keys for the bastions.** ssh applies the option to the last hop only,
  so a jump host is still asked about the old way.
- **The remote cursor's shape.** The canvas shows this machine's arrow; RDP sends the
  remote pointer (an I-beam over text) and a CSS cursor could show it.
- **A remembered graphics cache.** IronRDP can keep the pipeline's bitmap cache between
  sessions, so a reconnect to the same desktop paints faster.
- **More than one team vault.** Two customers on two shares. It is what spaces were,
  so not until someone has two.
