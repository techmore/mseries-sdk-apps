# mseries-greeter

Clean login screen for [greetd](https://git.sr.ht/~kennylevinsen/greetd): the first shell surface built the SDK way.

- `data/greeter.ui.json`: the screen in the `msdk` vocabulary, validated by `msdk check` (a test does this).
- `src/state.rs`: the logic as `State`, `Msg`, `update`, returning `Cmd` values. No GUI types.
- `src/greetd.rs`: the greetd wire protocol (length-prefixed JSON on `$GREETD_SOCK`).
- `src/main.rs`: GTK4 front end that renders that state. It is hand-written for now; the SDK renderer replaces it later.

Behaviour: the last user goes straight to the password step; "Not you?" returns to the user name; fingerprint or other PAM
prompts arrive as info messages and are shown and acknowledged; a wrong password shows a message and asks again; Restart and
Shut down are always available.

## Run it under greetd

`/etc/greetd/config.toml`:

```toml
[terminal]
vt = 1

[default_session]
command = "cage -s -- mseries-greeter"
user = "greeter"
```

`cage` (Arch `extra`) is the kiosk compositor for now. A greeter mode in `pantheon-compositor` replaces it so there is one compositor.
Environment: `MSERIES_GREETER_CMD` (session command, default `pantheon-rust-session`), `MSERIES_GREETER_STATE` (last-user file,
default `/var/cache/mseries-greeter/last-user`, must be writable by the `greeter` user).

## Not done yet

Touch ID via `pam_fprintd` is untested on hardware (the owner gates hardware changes); greeter mode in the compositor; accessibility
and keyboard-layout switcher; a PKGBUILD. Nothing here has been run under a real greetd.
