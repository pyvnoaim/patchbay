# TODO

What's next, then ideas. A line leaves this file when it ships (the changelog says it
then) or when it's decided against (say why in the commit).

## Next: the paid version

Written and parked in `git stash` as `paid: team licence ...` (`git stash list`, then
`git stash pop`). Free for one person; a team on a shared list needs a company licence,
€249 a year, perpetual fallback, checked offline. Blocked on the store, in this order:

1. Open a Lemon Squeezy store with the company licence as a yearly product. Licence
   keys off: the licence is ours, signed by `npm run licence "Acme GmbH" 2027-10-20`
   with `~/.tauri/patchbay-licence.key`, and sent by hand while orders are few.
2. Pick the domain. `patchbay.dev` is a placeholder in `site/index.html` and in the
   Licence pane's note.
3. Add the checkout link to the Licence pane, the site's Buy button (marked `CHECKOUT`,
   muted until then; the pricing is already live) and the README.
4. Have someone who knows licences read `LICENSE`. It is plain words, not a lawyer's.
5. Sign a real licence, paste it on a shared list, check the sidebar line goes, then
   `npm run check`, `npm test`, push and release.

Later, if orders aren't few: a Lemon Squeezy webhook into a function that runs the same
signing, so a licence arrives with the receipt.

## Ideas

Rough, unranked. Most are a `ponytail:` ceiling someone hasn't hit yet.

- **Shared host keys for the bastions.** ssh applies the option to the last hop only,
  so a jump host is still asked about the old way.
- **Offer to share a host key after a first connect**, not only from the device's
  detail, where few will look.
- **A fill-in blank in a snippet** (`journalctl -u {?} -f`), if ending the text where
  the variable goes turns out not to be enough. Not `{host}` or `{user}`: the shell is
  already on that machine, and a value from the file inside a command line means what
  lands on the prompt is no longer what the palette showed.
- **A snippets pane beside the session**, for the three you use all day. The palette
  is the only picker for now.
- **Your own devices in `~/.ssh/patchbay.conf`.** With two vaults they are named
  `me:<key>`, which is not a name ssh can be given, so the export leaves them out.
- **More than one team vault.** Two customers on two shares. It is what spaces were,
  so not until someone has two.
- **A web tab under a sheet or the palette** still goes blank while they are open. A
  snapshot of the page held in its place would hide the gap.
- **The RDP drive on Windows.** `drive.rs` is unix-only because IronRDP's backend is,
  so a Windows copy shares no folder in-app.
- **The trial file** beside the config restarts the trial when deleted. Move it to
  the keychain if that turns out to be common (paid version only).
