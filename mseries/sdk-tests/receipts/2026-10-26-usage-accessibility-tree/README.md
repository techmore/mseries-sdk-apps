# Usage accessibility tree diagnosis — 2026-10-26

Hosted ARM SDK run 40405291097 on 9dac12e failed the synthetic accessibility
probe. The retained tree shows a button named `Refresh` with role `button`; the
probe expected `Refresh usage`. Thus a button existed, but its exposed name did
not match the intended explicit name. No keyboard journey was completed. The
follow-up changes the visible button label to `Refresh usage` and needs a new
native test. The earlier private X display failure remains separate.
