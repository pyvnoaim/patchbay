# TODO

What's next, then ideas. A line leaves this file when it ships (the changelog says it
then) or when it's decided against (say why in the commit).

## Next: the paid version

Written and parked in `git stash` as `paid: Lemon Squeezy licence ...`
(`git stash list`, then `git stash pop`). Blocked on the store, in this order:

1. Open a Lemon Squeezy store and a product with licence keys on, three activations
   per key, no expiry.
2. Put the store id and product id into `STORE_ID` / `PRODUCT_ID` in `licence.rs`.
   At `0` every key is refused as "a different product".
3. Pick the domain. `patchbay.dev` is a placeholder in `site/index.html` and in the
   Licence pane's note.
4. Add the checkout link to the Licence pane, the site's Pricing band (now "Start the
   trial" points at GitHub releases) and the README.
5. Decide what happens to the public repo. The new LICENSE forbids copying the source,
   and a public repo with the check in plain sight is a fork away from free.
6. Activate a real key, deactivate it, let a trial run out with the clock moved
   forward, then `npm run check`, `npm test`, push and release.

Everyone on 0.1.27 starts a 14-day trial the moment they update, so none of this
ships half done.

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
- **Your own `[defaults]` in Settings.** With two vaults, Settings edits the team's;
  the ones in your own config are read but only hand-edited.
- **Your own devices in `~/.ssh/patchbay.conf`.** With two vaults they are named
  `me:<key>`, which is not a name ssh can be given, so the export leaves them out.
- **More than one team vault.** Two customers on two shares. It is what spaces were,
  so not until someone has two.
- **Route as a line.** The wide detail still draws the route top to bottom.
- **A web tab under a sheet or the palette** still goes blank while they are open. A
  snapshot of the page held in its place would hide the gap.
- **The RDP drive on Windows.** `drive.rs` is unix-only because IronRDP's backend is,
  so a Windows copy shares no folder in-app.
- **The trial file** beside the config restarts the trial when deleted. Move it to
  the keychain if that turns out to be common (paid version only).
