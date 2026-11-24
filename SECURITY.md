# Security

## Reporting a vulnerability

Please **do not open a public issue** for security problems. Use GitHub's private
[**Report a vulnerability**](https://github.com/yohannaftali/telepad/security/advisories/new)
form instead, with the Telepad version, your OS and steps to reproduce. You will get an
answer in the advisory thread; fixes are released as a new version, which running apps
offer through the in-app updater.

Only the latest release is supported.

## Security model

- Monitoring only runs read-only commands on hosts; `sudo` is only used as
  `sudo -n` (never prompts) for the BMC readings. Changes happen only through actions
  the user starts and confirms: Docker / BMC actions and Log info's Register / Unregister
  Systemd Journal (`sudo usermod -aG` / `gpasswd -d` for the login user, sudo asks for the
  password) in a terminal, and **Authorized keys**
  (reads `~/.ssh/authorized_keys` to list the keys — read-only; adds or removes one line in the login user's `~/.ssh/authorized_keys` after the user
  types `add` / `revoke`; it refuses to revoke the key Telepad itself logs in with).
- **Log info** reads SSH logins with plain `journalctl` (no sudo) only when its page opens or on
  Refresh. The `systemd-journal` group it can add lets the login user read every system log
  (not only SSH), which may hold sensitive text; it gives no write access and no root.
- **Open database…** reaches a database only through `ssh -N -L` over the host's normal
  login (`BatchMode=yes`); no database port is opened and nothing on the host is changed.
  **Import…** (#139) adds rows only after the user saw the mapping, the first rows and the
  exact `INSERT` statements and clicked Import, in one transaction, never on Read-only
  connections; a SQL file is limited to `INSERT INTO` the chosen table. Exports write only
  to the file the user picked.
  The one exception is **My Computer**: its own databases are opened on `127.0.0.1` directly
  (no SSH); any other address is refused.
- **My Computer** reads this computer read-only and without administrator rights (system
  information, `netstat -ano`, `reg query`, `docker` / `podman` on Windows; the host script
  run locally with `sh -s` on Linux / macOS, `sudo -n` only as on hosts).
  Data and structure change only after the user saw the exact SQL and clicked Run
  (`DROP` / `TRUNCATE` also need the table name typed); values are bound parameters and
  names are quoted. **Read-only** makes the server refuse writes for that connection.
  Database passwords are kept in memory, or — when the user ticks **Save password** — in
  the OS password store (Windows Credential Manager, macOS Keychain, Linux Secret
  Service); never in `telepad.db` or another file. Only the database user name is read
  from a container's environment, filtered on the host.
- **Open files…** copies files only when the user clicks Upload / Download, presses F5 or
  drags files, over the host's normal SSH login (SFTP subsystem; `scp -O` or rsync over the
  same login as fallbacks; never `sudo`). An existing file is replaced only after the user
  chose Overwrite (or Newer only); copies go to a `.telepad-part` file that is renamed when
  complete. Uploads never copy Windows' file modes, so files are not made world-writable;
  rsync runs a dry run first and never deletes. New / rename / delete / permissions run
  only from the file window's dialogs (delete after a confirmation that lists the items),
  with the login user's rights (no `sudo`); names are checked to be one path element,
  renames never replace a file, and recursive delete / permission changes never follow links.
  The editor saves a file only on Save (Ctrl+S, or Save in the unsaved-changes question),
  via `<name>.telepad-part` and a rename, keeping the file's mode; it refuses to write over
  a file that changed since it was loaded unless the user clicks Overwrite.
- New host keys are accepted on first connect (`StrictHostKeyChecking=accept-new`);
  a **changed** host key is rejected, as with plain ssh.
- Host fields are validated before they are saved, because the database is rendered
  into an ssh config; values are passed to ssh as separate arguments, never through a shell.
- Updates are downloaded only from this repository's GitHub release downloads and
  checked against the published SHA-256 before anything is replaced.
- Telepad stores no SSH passwords or keys: it uses the OpenSSH client and the key files
  your ssh config already names (database passwords: see above). `telepad.db` holds host names, addresses, users and
  key *paths*, so treat it like an ssh config and do not share it.
