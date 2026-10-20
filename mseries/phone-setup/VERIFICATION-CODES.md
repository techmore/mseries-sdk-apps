# Verification codes from iPhone texts

Install `bash mseries/install-verification-codes.sh` after BlueFerry is working.
This adds **Verification Codes** to Applications and a separate user service.
It preserves the running Messages window and Bluetooth services.

Focus a verification-code field. A recent matching code can appear near the
pointer. Press the shortcut printed by the installer (normally **Shift+Super+V**)
to activate the offer, then **Enter** to use it. **Esc** dismisses. The popup shows
the selected field and app. Existing shortcuts are compared as parsed accelerators;
aliases and modifier order count as the same binding. If occupied, the installer
tries Shift+Super+O, then Ctrl+Alt+O; Applications remains available if all are busy.

Native apps that expose AT-SPI EditableText support **Use Code**. Insertion replaces
that exact, revalidated code field and does not press Enter or submit the form.
Passive offers leave partially entered fields alone; the explicit shortcut can
replace a value in a clearly labelled code field. Generic password and message
fields are rejected. Split one-character fields require manual paste.

**Brave currently supports offers and Copy Code, not direct insertion.** Its
Linux accessibility bridge exposes field metadata and Text but not EditableText.
The popup therefore offers **Copy Code** with Enter as its action, closes, and
lets the owner paste. No global typing or automatic clipboard change is used.
Direct browser filling needs a companion extension and is still open work.
This limitation was reproduced with Brave 155.1.97.59 in a private profile;
[Chromium's interface registration](https://chromium.googlesource.com/chromium/src/+/c213438d949dc89f3d8d5df3dec27dcc01d7d027/ui/accessibility/platform/ax_platform_node_auralinux.cc)
also omits EditableText. No Chromium implementation is copied here.

The helper uses upstream BlueFerry's deterministic OTP extraction and its
existing backend, not a second phone connection. It requests at most 20 recent
inbox messages at startup or when the backend event count changes. Offers expire
after five minutes and live only in RAM. This helper writes no message bodies,
codes or recipient metadata to disk/logs; upstream history retains its existing
policy. Only an explicit **Copy Code** action writes the clipboard. Consumed
offers are suppressed for the remaining lifetime of the process. Domain-bound
SMS codes require the exact HTTPS host; embedded-frame bindings are withheld.
Unbound SMS codes have no domain assertion and the destination remains visible.

The popup uses GTK 4/Granite 7 and follows the desktop's light/dark preference.
It runs under XWayland to position near the pointer; Messages remains native
Wayland. `xdotool` reads pointer/display geometry and moves/activates only the
helper's own window. Runtime code insertion uses the field's accessibility API.
No desktop compositor, panel, keyboard driver or browser debug interface is modified.

## Checks and remaining qualification

Run `python3 mseries/phone-setup/check_verification_codes.py` for synthetic-only
model, keyboard-to-native-field, expiry, password rejection and shortcut
preservation checks. `--browser` additionally launches an isolated browser and
checks actual clipboard copy with unchanged DOM input/submit counters. It needs
Brave/Chromium and Python `websockets`. `--screenshots DIR` renders light/dark
and copy-only cards. Tests create a private home, settings backend, D-Bus and
display; they never query the owner's phone or browser profile.

Live owner sign-in insertion, pointer placement/focus on the running compositor,
session restart and all other apps/sites remain qualification work. Synthetic
field and private-browser passes do not establish universal autofill support.

Undo with `bash mseries/install-verification-codes.sh --uninstall`. It removes
only this helper's files/service and its still-owned shortcut, retaining unrelated
bindings, phone pairing, history and the Messages window.
