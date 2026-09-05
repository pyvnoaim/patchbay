# Teams: one shared list

Plan for a company or team working on the same `patchbay.toml`. Written 2026-09-04,
revised and built 2026-09-05.

## The pitch

**A device list is a file in a folder you already share.** Put `patchbay.toml` on the
team drive, Dropbox, OneDrive or an SMB share, point Settings ▸ Team at it, and
everyone sees the same list and the same folder notes. Text, so grep and scripts
read it too. Nothing to install on a server, nothing to license, and the file holds
no credentials, so sharing it leaks hostnames and nothing more.

That is the whole case against Royal TS, whose team story is Royal Server: a
licensed Windows service, a document password, encrypted blobs in XML nobody can
review, and a seat licence per client. It exists to guard stored passwords we don't
have.

The switch is already built: one person imports the `.rtsz`, puts the file on the
share, everyone points at it. That is step one of the docs, not a footnote.

## The shape

One setting that says where the list lives, a reload when it changes, and nothing
else. Almost everything a team needs is already here: every write re-reads the file
from disk before editing, lands as temp+rename, keeps comments, and the window
reloads on every focus.

## Steps, in order

1. **Split "mine" from "the list".**
   - `Settings` in `config.rs` gets `list: Option<String>`.
   - `list_path()` in `patchbay.rs`: `[settings].list` from my own config if set
     (with a leading `~` expanded), else my own config. Test both.
   - Jacks, `[defaults]`, `[folder]` notes and every writer of those (`save_jack`,
     `delete_jack`, `set_note`, `rename_group`, `delete_group`, defaults, import)
     swap from `config_path()` to `list_path()`. `load_jacks()` in `commands/mod.rs`
     is the one reader nearly everything goes through, so swapping it covers
     sessions, remote, web and deep links at once. The stragglers that read the
     path directly: `notes()` in `commands/jacks.rs`, `config_path` and
     `open_config` in `commands/settings.rs`.
   - **With `list` set, a missing file is an error, never an empty list.** Today
     `load` returns an empty list on NotFound, `read_doc` starts from the template,
     and `write_doc` runs `create_dir_all`, because a missing own config is a first
     run. On an unmounted share that paints an empty window, and the next save
     writes a fresh one-jack file into a directory it just made; on macOS that
     directory sits at `/Volumes/team/` and the share mounts beside it as `team-1`
     from then on. So every reader and writer of the list refuses NotFound with
     "the list at X is not there", and nothing creates directories on that path.
   - `[settings]`, `[colors]`, `web_trusted`, `rdp_known_hosts`, `logs/` and
     `fold_spaces_at` stay on my own file. They are this machine's answer, not part
     of the list.
   - Solo users see no change: still one file.

2. **Settings ▸ Team.**
   - One text field for the path, same shape as the ssh-config import path. No
     dialog plugin, none needed. A UNC path works on Windows.
   - Saving a path whose file doesn't exist copies your current list there, so the
     first person seeds the share: jacks, `[defaults]` and `[folder]` notes only.
     The seed is the one write allowed at a missing path, and only when the parent
     directory already exists: a share that is away looks the same as a share that
     was never seeded, and the difference is the directory being there.
     `[settings]` and `[colors]` never leave your machine. `[defaults]` goes as it
     is, and the sheet says so in one line: a `user` there is now everyone's.
     Stripping it silently would change how the seeder's own devices connect. The
     file starts with the same commented key template a fresh config gets, because
     a colleague will open it by hand.
   - Settings shows both paths, and anything that opens the config for hand-editing
     opens the list. Pointing at a file that exists shows *that* list;
     your own devices stay in your file, dormant, and come back when you clear the
     setting. That is how you leave a team, and it is worth one sentence in the docs.

3. **Notice changes.**
   - No thread and no event. A `list_stamp` command returns the file's mtime and
     size; its own thirty-second `setInterval` in `boot.js` calls `load()` when
     either differs. Not inside `refreshProbes`, which returns early when probing is
     off or the window is unfocused. Differs, not newer: OneDrive keeps the source
     machine's mtime and clocks disagree. Focus reload covers the rest. A
     two-second interval is the upgrade if someone finds thirty too slow. One
     in-flight flag on the interval: a stat on a dead share hangs for a minute, and
     without it the ticks stack up as blocking tasks behind each other.
   - `renderDetail` already refuses to redraw a pane being typed in, and the sheet
     is a form outside the render path, so a reload mid-edit is safe.

4. **Same-device edits.** `jacks()` hands out a `stamp` per jack, a hash of its raw
   TOML table. The sheet sends it back with the edit and `config.rs` refuses when
   the current table hashes differently: "web01 was changed by someone else, reload
   and try again". Same for `set_note`, since a note is the likeliest thing two
   people both touch. Not for drags and folder renames, which rewrite one field and
   are already safe. One hash, one comparison, one test. On OneDrive the race is not
   milliseconds, it is the sync delay, so this is in scope rather than deferred.
   A refusal reloads the list and leaves the sheet open with the edit in it and
   the *new* stamp, so nothing is retyped and the second save goes through. That
   second save is a knowing overwrite of the colleague's edit, without showing what
   it was; the pill says "saving again replaces their change". Without the new
   stamp the retry is refused forever.
   Delete then Undo restores what was deleted over a colleague's edit in between;
   the pill lasts seconds, so accepted.

5. **A share that is away.** A laptop off the VPN, or a NAS that is down, must not
   be an empty window.
   - `load()` failing keeps the list the window already has and says so in a pill.
     The state is already there; this is one branch in `load()`.
   - A save while the share is away fails with the OS's error in the sheet. No
     queue: the user retries when they are back, and the list on screen is still
     what they were looking at. On Windows the same error comes from a file
     another process holds open, a sync client mid-upload or a colleague's editor,
     because `rename` over it is refused. The docs name that, or it reads as a bug.
   - Every command that touches the list becomes `async fn`. A sync Tauri command
     runs on the main thread, and a stat on a dead SMB share hangs for tens of
     seconds, so today a config on one would freeze the window; a list on one must
     not.

6. **Docs and changelog.**
   - Import your Royal document, put the file on the share, point Settings ▸ Team
     at it. Colleagues point at the same path.
   - `user` and `key` for a *shared* account (`root@nas`, `~/.ssh/acme`) belong in
     the list; `~` expands per machine. A personal account on a shared server goes
     in `~/.ssh/config`, which overrides nothing in the list but supplies what it
     leaves out.
   - A share that is read-only for someone is a read-only list for them. No roles
     to build: the save fails with the OS's permission error, shown in the sheet.
   - Who can edit can point `prod-db` at a host they own. Known hosts catches an
     existing name moved, not a new one. That is the trust a shared Royal document
     extends too, minus the passwords; say so rather than pretend otherwise.

## Security, specifically

- The list is always "a file someone else wrote", and every guard already assumes
  that: `dest()`, `forward_arg`, the url check, `esc()`, the `.rdp` control-character
  reject.
- `[settings]` is never read from the list file, so a shared file can't redirect
  where settings come from or point `list` somewhere else.
- With `write_ssh_config` on, a colleague's jack becomes lines in your
  `~/.ssh/patchbay.conf`. `to_ssh_config` already runs names through `plain_token`
  and forwards through `forward_arg`, the same checks that guard argv, so nothing new
  is needed there.
- Who may edit is the share's permissions. Deliberate: an app-level lock on a file
  with nothing secret in it is theatre.
- Nothing is fetched over the network for this. The list is a path, never a URL.

## Known gaps

- **Sync clients are not a shared filesystem.** "Edits to different devices both
  survive" holds on SMB or a mapped drive, where the re-read sees the other write.
  On OneDrive and Dropbox it holds only when the sync lands before the second save;
  two people editing in the same minute get a conflicted copy beside the file, and
  one edit is silently gone from the list. `list_stamp` lists the folder for a
  conflicted-copy name too, and a pill says so with Reveal. No merge, never silent.
  The docs say it plainly: a team that edits at the same time puts the file on a
  share, a team that mostly reads can use a sync client.
- **No write queue.** A save while the share is away fails and the user retries.
  A queue is the thing that silently overwrites a colleague an hour later.
- **No history.** The share's own versioning (Dropbox, OneDrive and most NAS
  snapshots have one) is the undo. Not ours to build.
- **A homelab beside the customers.** One list is the rule, so a personal device
  goes in a folder of your own in the shared list, or in `~/.ssh/config`, or is
  typed into the palette. This is the first complaint to expect. A private list
  merged over the shared one is the fix, and it costs every writer knowing which
  file a jack came from; not until asked.
- **Per-person overrides on the shared list.** `~/.ssh/config` already is that layer
  for user, key and ProxyCommand.
