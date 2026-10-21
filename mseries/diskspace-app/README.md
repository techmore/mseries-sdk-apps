# Disk Space (Rust + Granite 7)

An ncdu-style disk usage analyzer for the Pantheon stack, with a map instead of a text list. Pick a filesystem on the
left, then see where the space went as **cards** (a squarified treemap, big folders show what is inside them) or as a
**circle** (a four-ring sunburst around the current folder), and give space back from the list on the right.

```
rust/mseries-diskspace/        the GTK 4 + Granite 7 app (granite-rs); scanner, layout, drawing, window, tests
mseries/diskspace-app/         install.sh (dev install), test.sh (everything CI runs), this file
```

## Using it

- **Left:** Home, Choose Folder…, and every mounted block or network filesystem with free space and a usage bar
  (orange from 85% full, red from 95%). Clicking one scans it.
- **Middle:** breadcrumb (click any part to jump), then the map. Click selects, Ctrl+click adds, double-click enters a
  folder; in the circle, clicking the centre goes up. Hover for the full path, size, share and type. Colour is the file
  type (folder, image, media, archive, disk/VM image, package, document, build output).
- **Right:** the current folder's contents, sortable by size, name, item count or modified date (and reversible), with a
  bar for each item's share. Selection is shared between the list and the map.
- **Actions** (bottom right): Copy path, Show in Files, **Move to Trash** (recoverable; the space returns when the Trash is
  emptied, and the toast offers Open Trash) and **Delete…** (permanent, asks first, frees the space now).
- **Keys:** Delete = Move to Trash, Backspace = up, Ctrl+R = scan again, Esc = clear selection.
- **Menu:** *Include other filesystems* also counts mounts and Btrfs subvolumes found inside the folder.

## What it measures

Allocated disk space (`st_blocks`), the number `du` and `df` use, not file length. A hard-linked file counts once,
symlinks are not followed, `/proc`, `/sys` and `/dev` are never entered, and by default the scan stays on one filesystem.
That matters on this machine: `/` and `/home` are separate Btrfs subvolumes of one pool, so scanning **System** leaves out
`/home` and says so in the status bar ("not counted (other filesystem)"). Folders you cannot read are counted and reported
rather than silently skipped. Btrfs snapshots share extents, so sizes inside them are not individually reclaimable
(`mseries/STORAGE-PLAN.md`); judge recovery by the free space the sidebar reports after deleting.

`mseries-diskspace --scan FOLDER` prints the total and the largest children without a window; `test.sh` uses it to check
the scanner against `du -sx --block-size=1`.

## What it will not remove

The scan root, `/`, anything directly under `/`, your home folder and any folder containing it, and other filesystems.
The buttons are disabled for those, with a tooltip saying why. Everything else is removed as the signed-in person, so the
usual permissions apply. Nothing is removed without a click, and permanent delete asks first.

## Install, test

```bash
bash mseries/diskspace-app/install.sh      # builds, installs under /usr/local, asks for sudo; --uninstall removes it
bash mseries/diskspace-app/test.sh         # unit tests, scan vs du, headless render under Xvfb, staged install
MSERIES_DISKSPACE_SHOTS=/tmp/shots bash mseries/diskspace-app/test.sh   # keep light/dark screenshots
```

The render test opens the real window on a generated folder, clicks cards and the circle centre like a mouse would,
then trashes one folder and deletes another and checks the list, totals and files on disk (it uses a private XDG data
directory, so the real Trash is untouched). It never touches the owner's display. No Wingpanel, dock or compositor is
restarted by install.

## Limits (v0.1)

- The scan has no live preview: a progress count shows while it runs and the map appears when it finishes (a full scan of
  `/` here takes well under a second; a large spinning disk or network share will take longer, and Cancel works).
- The tree lives in memory (about 100 bytes per entry), so scanning tens of millions of files will use gigabytes.
- Trashing needs a user-visible filesystem: GLib refuses system-internal mounts such as `/tmp`, and the app shows that
  message; use Delete… there.
- No translations, no keyboard navigation inside the map itself (the list is fully keyboard accessible and mirrors it),
  no screen reader pass, no file-type filter or search yet.
- Verified on this machine under Xvfb only; not yet run on the owner's Pantheon session.
