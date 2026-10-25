# TODO

What's next, then ideas. A line leaves this file when it ships (the changelog says it
then) or when it's decided against (say why in the commit).

## Next: the paid version

Written and parked in `git stash` as `paid: team licence ...` (`git stash list`, then
`git stash pop`). Free for one person; a team on a shared list needs a company licence,
€249 a year, perpetual fallback, checked offline.

The licence is a minisign-signed file beside the shared list (`licence.rs`, `npm run
licence`). Nothing is gated on it: an unlicensed list works the same and the sidebar says
so. Its `updates_until` is held against the build date, so a team that stops paying keeps
a licensed copy. The stash is on 0.1.38; a `git stash pop` on a newer main may conflict in
`commands/jacks.rs`, `ui/edit.js` and `CHANGELOG.md`.

Blocked on the store, in this order:

1. Open a Lemon Squeezy store with the company licence as a yearly product. Licence
   keys off: the licence is ours, signed by `npm run licence "Acme GmbH" 2027-10-20`
   with `~/.tauri/patchbay-licence.key`, and sent by hand while orders are few.
2. Pick the domain. `patchbay.dev` is a placeholder in `site/index.html` and in the
   Licence pane's note.
3. Add the checkout link to the Licence pane, the site and the README. The site says
   "coming soon" until then: bring the €249 card, its small print and the `CHECKOUT`
   Buy button back from the commit that took them down.
4. Have someone who knows licences read `LICENSE`. It is plain words, not a lawyer's.
5. Sign a real licence, paste it on a shared list, check the sidebar line goes, then
   `npm run check`, `npm test`, push and release.

Later, if orders aren't few: a Lemon Squeezy webhook into a function that runs the same
signing, so a licence arrives with the receipt.

## Next: IronRDP back from the fork

The graphics pipeline needs IronRDP's main branch, pinned in `src-tauri/Cargo.toml` to
`pyvnoaim/IronRDP`: that main plus two speedups sent upstream as Devolutions/IronRDP#2216
and #2217. Once both merge, point the pin at Devolutions' repo; once a release carries the
pipeline, go back to crates.io.

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
