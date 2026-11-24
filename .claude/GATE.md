# The push gate for patchbay

When I say **push**, that is not just `git push`. It is the whole procedure, and it is
the *only* time anything is committed:

1. **Take everything open.** Uncommitted changes, staged and unstaged, new files, and anything already committed but unpushed. The review covers the lot, not just the last thing worked on.
2. **Review it all**, in these four passes:
   - **Bugs and correctness**: especially in `patchbay.rs`, where a wrong argv is a connection that fails somewhere else entirely.
   - **Security**: problems *and* improvements. Anything reaching a shell, the trust store, a `style` attribute, the desktop opener, or a file someone else wrote.
   - **Performance**: problems *and* improvements. Something that got slower, an extra round trip on a hot path, work done before the user sees anything.
   - **Simplification**: what can be deleted, reused, or replaced by something already here.
3. **Run `npm run check` and `npm test`**: the whole crate, not one module's `#[cfg(test)]`.
4. **Clean → commit and push.** Sensible commit messages, split into separate commits when the changes are unrelated.
5. **No release.** A push lands under `## Unreleased` and stops there. Each release is an update pill and a restart for everyone, and 85 of them in ten weeks read as churn to someone paying for it. Say how many entries are waiting and since when, one line. A bullet goes in only if a user would notice it: something they can now do, or something that stopped breaking. Alignment, spacing and wording tweaks share one "Small fixes on the device page." at most.
6. **Not clean → fix it, then push.** Fix what the review turns up in the same pass and rerun step 3. Stopping to hand the findings back only buys a round trip. List every finding and what was done about it, one line each, so nothing is fixed silently. A finding that would change behaviour someone relies on is still my call: name it and leave it.

Findings first, one line each. Don't push a "probably fine".

## "release"

When I say **release**: run the gate above if anything is unpushed, then `npm run bump` (next patch), commit `release x.y.z` as the last commit and push `main`. There is no tag to push. CI builds macOS, Windows and Linux and publishes, and every installed copy offers the update on its next check. Before bumping, reread `## Unreleased` as one release: merge bullets about the same thing, drop one a later bullet undoes.

Suggest a release, don't cut one, when `## Unreleased` holds a fix for something the last release broke, or has waited a week.
