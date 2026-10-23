# s02 Work Mode settings (easy to medium)

Golden reference: behaviour in [../../work-mode/README.md](../../work-mode/README.md).

A settings page: a master switch "Work Mode", a weekday schedule (checkbox per day, start and end time
pickers), a wallpaper choice (two preview cards: Work, Default) and a "Apply now" button. Changes apply
immediately and are stored in GSettings. Switch off dims the other controls.

Capabilities: gsettings (own schema), D-Bus session bus (to read the current wallpaper).

Acceptance:
1. Master switch off disables every control below it; on enables them.
2. End time earlier than start time shows an inline error and does not save.
3. Selecting a wallpaper card marks it selected (not by colour alone).
4. "Apply now" shows a spinner on the button until the call returns, then a toast.
5. Values survive restart.
6. Fully keyboard-operable; the cards are focusable and selectable with Space.
