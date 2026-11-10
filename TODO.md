# TODO

What's next, then ideas. A line leaves when it ships (into the changelog) or is decided
against (why goes in the commit).

## Now: try the shipped work in the real app

Released and unit-tested, never run in the window. `npm run dev` and tick these off
before the next push.

- [ ] **VNC tab** on a Linux desktop, Proxmox/QEMU console or KVM: typing (with Shift and
      ⌥), the remote cursor's shape, clipboard both ways (copy there, paste here, then
      copy here, click in, paste there).
- [ ] **VNC password**: a server that wants one asks. Remember works. A wrong saved one is
      asked again still ticked. A Mac with only Apple's sign-in opens in Screen Sharing.
- [ ] **RDP handoff** for 2008 R2: opens outside the window.
- [ ] **RDP sound**: a video or a system sound plays, in sync. Does it start at once or
      only after the first click in the desktop?
- [ ] **Settings**: a switch takes effect at once. Theme, font size and the shared list
      save on leaving the field. Closing with × saves a half-typed field. An invalid
      snippet keeps the sheet open, closing twice discards it.
- [ ] **The 0.1.63 shell**, in light and dark: the device page, sidebar, quick connect,
      the device form (More, Advanced, the ssh preview) and Settings at a narrow and a
      wide window.
- [ ] **Background tabs**: output in another tab lights its dot. A drop and a bell there
      raise one notice and one Dock bounce, not one per line.
- [ ] **Haptics** on a Force Touch trackpad: a device dragged onto a folder ticks. The
      sidebar edge settles on its default width with a tick.
- [ ] **Telnet**: a device with `telnet = 23`, without `telnet` installed (the curl
      fallback): login prompt, typing, echo, ^C. Once behind a jump host.
- [ ] **Serial**: a console cable at `/dev/cu.usbserial-…`, 9600 and 115200.
- [ ] **Split panes**: ⌘D beside a shell, an RDP tab and a web tab. Drag and double-click
      the divider. ⌘[ / ⌘] through panes. ⌘W one pane. "Stack the panes" and
      "Separate into tabs" from the tab's menu. A second shell of the same device.
- [ ] **VoiceOver** (⌘F5): the tree reads names and status, tabs read as tabs, icon
      buttons have names, the menu key opens a device's menu.
- [ ] **Connected for**: a device's page says "opened now, for N min" with a tab open and
      "N minutes ago" after. A colleague on a team device shows "(N min)" in the tooltip and
      On now. A colleague on 0.1.71 still shows, without minutes.
- [ ] **Licence pane**: with one vault it says free. With a shared list it shows no
      licence needed yet. A fresh `npm run licence` paste is accepted and removable.
- [ ] **Local shell**: "local" in ⌘K opens a tab, in ⌘D opens beside. `brew` works in it
      (the login PATH). `exit` then ⏎ starts another. It reopens after a relaunch.
- [ ] **Device row toggle**: with a session open, clicking its row shows the page, clicking
      again goes back to the session.
- [ ] **Snippet presets**: "Add N common ones" in Settings ▸ Snippets adds them to Mine,
      they survive closing and reopening, and `restart` asks for a unit.

## Next: the paid version

Built: `licence.rs`, Settings ▸ Licence, `npm run licence`. A shared list needs a
company licence, €249 a year, checked offline. An ended one keeps the list open a week
under a "licence ended" line, then stops. Own devices always open. Nothing asks while
`licence::ON_SALE` is false.

Left, in this order:

1. Open a Lemon Squeezy store, the company licence as a yearly product, its licence keys
   off: ours are signed by `npm run licence` and sent by hand while orders are few.
2. Buy `patchbay.page` (free on Cloudflare on 2026-11-05) and point it at `site/`. The
   site already uses it.
3. Add the checkout link to the site and the README (the €249 card's Buy button, and
   drop its "on sale soon"), and a Buy link in Settings ▸ Licence.
4. Have someone who knows licences read `LICENSE`. It is plain words, not a lawyer's.
5. Sign a real licence, paste it on a shared list with `ON_SALE` on in a dev build, and
   check the sidebar line goes. One that has ended closes the list after the week.
6. The launch is 1.0.0 (`npm run bump 1.0.0`), never a reset to 0.1.0: the updater only
   moves up. Turn `ON_SALE` on in the same commit. Make `LICENSE`'s last line "Versions
   before 1.0.0", but only if nothing after 0.1.58 has shipped under Perimeter.
7. Once 1.0.0 is published, delete the 0.1.x GitHub releases (`gh release delete <tag>
   --cleanup-tag`). Not before: installed copies update through the latest release's
   `latest.json`, so 1.0.0 has to be live first. Only new downloads stop. Existing copies
   and git history stay under Perimeter.

Later, if orders aren't few: a Lemon Squeezy webhook running the same signing, so the
licence arrives with the receipt.

## Next: IronRDP back from the fork

`src-tauri/Cargo.toml` pins `pyvnoaim/IronRDP`: IronRDP main plus two speedups sent
upstream as Devolutions/IronRDP#2216 and #2217. Once both merge, pin Devolutions' repo.
Once `ironrdp-egfx-native` and a new `ironrdp` reach crates.io (the pipeline did on
2026-10-23), go back to crates.io.

## Ideas

Rough, unranked. Most are a `ponytail:` ceiling someone hasn't hit yet.

- **Shared host keys for the bastions.** ssh applies the option to the last hop only,
  so a jump host is still asked about the old way.
- **A remembered graphics cache.** IronRDP can keep the pipeline's bitmap cache between
  sessions, so a reconnect to the same desktop paints faster.
- **More than one team vault.** Two customers on two shares. It is what spaces were,
  so not until someone has two.
