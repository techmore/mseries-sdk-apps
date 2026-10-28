# granite-rs → Granite 9 bindings (prototype patch)

`granite-rs-granite9.patch` is a prototype port of
[davidmhewitt/granite-rs](https://github.com/davidmhewitt/granite-rs) from
Granite 7 to Granite 9 (elementary OS 9). It was rescued from an unpushed
working copy (`/tmp/mseries-granite-rs9`) on 2026-10-28; an identical copy also
sits in the non-git folder `~/Projects/granite9-rust-prototype`.

- Base: granite-rs `ecb96c69aed40abf45458df9460809d188160079` ("chore: bump GTK and GLib versions (#23)").
- Contents: `Granite-9.0.gir` plus `fix9.sh` (xmlstarlet clean-up: drops Gee,
  sets `libgranite-9.so.9`), regenerated `granite/src/auto` and `granite/sys`
  (links `libgranite-9`, `system-deps` `granite-9`), a `gtk_v4_10` feature,
  removed Settings{Page,Sidebar}/SimpleSettingsPage, and a `gir-files`
  submodule bump to `31e51e34`.
- Status: unverified prototype — not built or tested here.

Apply with:

```sh
git clone https://github.com/davidmhewitt/granite-rs && cd granite-rs
git checkout ecb96c69aed40abf45458df9460809d188160079
git apply /path/to/granite-rs-granite9.patch
git submodule update --init
```
