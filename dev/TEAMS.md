# Teams: one shared list

Plan for several people using, working with and editing the same `patchbay.toml`.
Written 2026-09-04. Not built yet.

## The shape

One setting that says where the list lives, a poll that notices it changed, and
nothing else. Almost everything a team needs is already here: every write re-reads
the file from disk before editing, lands as temp+rename, keeps comments, and the
window reloads on every focus. The file holds no credentials, so sharing it leaks
hostnames and nothing more. That is the whole security story against Royal TS,
whose team feature exists mostly to protect stored passwords we don't have.

Royal TS needs Royal Server and a document password. We need a folder two people
can both see: a git checkout, Dropbox, OneDrive, an SMB share. Git is the better
one, because it gives history and review (a PR to add a customer) for free.

## Steps, in order

1. **Split "mine" from "the list".**
   - `Settings` in `config.rs` gets `list: Option<String>`.
   - `list_path()` in `patchbay.rs`: `[settings].list` from my own config if set
     (with a leading `~` expanded), else my own config. Test both.
   - Jacks, `[defaults]`, `[folder]` notes and every writer of those (`save_jack`,
     `delete_jack`, `set_note`, `rename_group`, `delete_group`, defaults, import)
     swap from `config_path()` to `list_path()`.
   - `[settings]`, `[colors]`, `web_trusted`, `rdp_known_hosts`, `logs/` and
     `fold_spaces_at` stay on my own file. They are this machine's answer, not part
     of the list.
   - Solo users see no change: still one file.

2. **Settings ▸ Team.**
   - One text field for the path, same shape as the ssh-config import path. No
     dialog plugin, none needed.
   - Saving a path whose file doesn't exist copies your current list there, so the
     first person seeds the share. Clearing it goes back to your own file.

3. **Notice changes while the window is focused.**
   - A thread in `main.rs` stats `list_path()` every two seconds and emits
     `config:changed`; `boot.js` calls `load()`.
   - Poll, not `notify`: no new dependency, and Dropbox, SMB and OneDrive don't
     deliver file events reliably.
   - `renderDetail` already refuses to redraw a pane being typed in, and the sheet
     is a form outside the render path, so a reload mid-edit is safe.

4. **Docs and changelog.**
   - Put the file in a git repo or a synced folder, point Settings ▸ Team at it.
   - Leave `user` out of the team file. Each person's `~/.ssh/config` says who they
     are; a `user` in the list overrides it for everyone.
   - A share that is read-only for someone is a read-only list for them. No roles
     to build: the save fails with the OS's permission error, shown in the sheet.

## Security, specifically

- The list is always "a file someone else wrote", and every guard already assumes
  that: `dest()`, `forward_arg`, the url check, `esc()`, the `.rdp` control-character
  reject.
- `[settings]` is never read from the list file, so a shared file can't redirect
  where settings come from or point `list` somewhere else.
- Review `to_ssh_config` in the push pass: with `write_ssh_config` on, a colleague's
  jack becomes lines in your `~/.ssh/patchbay.conf`.
- Who may edit is the share's permissions or git branch protection. Deliberate: an
  app-level lock on a file with nothing secret in it is theatre.
- Nothing is fetched over the network for this. The list is a path, never a URL.

## Skipped, and when to add

- **Locking, or an optimistic "changed under you" check on save.** Two people
  editing the same device in the same seconds lose one edit; git or a conflicted
  copy is the answer. Add an mtime check on save if it bites.
- **A personal list beside the team list.** One list is the rule that just cost us
  spaces. A home NAS goes in a folder of your own, or in `~/.ssh/config`.
- **A sync service, or fetching the list over https.** Git is a better one than
  anything we'd write.
- **Per-person overrides layered on the shared list.** `~/.ssh/config` already is
  that layer for the things that differ per person (user, key, ProxyCommand).
