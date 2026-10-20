# One macOS visit: native Linux messaging and optional Machine Lab

This kit supports the ARM64 Bubbles build already installed on Linux. The native
Connect iPhone/BlueFerry build is also installed, but Bluetooth message access is
still denied. Exporting real Mac hardware is a separate Bubbles activation path.
No Apple Account password is needed during this export. No VM is needed by Bubbles.

## 1. Download on macOS

Boot your existing **native macOS partition**. Open this GitHub page in Safari.
Open Terminal and run the download command linked in the parent phone-setup README.
It creates `~/Downloads/MSeries-Mac-Visit`, downloads our scripts at a fixed Git
revision and the [official Mac Hardware Info v1.2.0 release](https://github.com/OpenBubbles/Mac-Hardware-Info/releases/tag/v1.2.0),
checks its archive SHA-256 against our retained download, and opens the folder.
The hash verifies consistency with our download, not an independent publisher signature.

## 2. Export and verify before rebooting

1. Open **Mac Hardware Info.app** from that folder. If blocked, use System
   Settings → Privacy & Security → Open Anyway for this app after reviewing its
   official source. Do not disable Gatekeeper globally.
2. Turn **Prevent sharing OFF**. Click **Copy Activation Code**.
3. In Terminal run:

   ```sh
   /bin/bash "$HOME/Downloads/MSeries-Mac-Visit/Save-Hardware-Code.command"
   ```

4. Press Return when prompted. Wait for **Saved mac-hardware.txt**. The helper
   rejects short sharing codes and checks the full OABS header without printing
   your code. It also collects the installed AVPBooter when available for the
   separate VM experiment. Keep the original export if the file already exists.
5. Do not start Apple login or 2FA yet. That happens in Bubbles on Linux.

The full export is reusable; it is not the 48-hour hosted sharing code. The Mac
can be shut down afterward. Successful export is not yet proof of Apple activation.

## 3. Make the private files accessible from Linux

**Preferred:** copy the whole `MSeries-Mac-Visit` folder onto a USB/external drive
readable by Linux and macOS. exFAT works; do not format an existing drive just for
this task. Check in Finder that the destination contains a nonempty
`mac-hardware.txt`. Keep both copies until activation succeeds.

**Without a drive:** use an encrypted archive in your own cloud drive, then
retrieve it from its website after returning to Linux. With Homebrew available:

```sh
brew install sevenzip
cd "$HOME/Downloads/MSeries-Mac-Visit"
7zz a -t7z -mhe=on -p "$HOME/Desktop/MSeries-private.7z" mac-hardware.txt
```

Enter a strong archive password when prompted and remember it outside the archive.
Upload **only the encrypted .7z** to your private cloud account, never the raw
export. On Linux we can extract it locally and remove the cloud copy afterward.
Do not upload the export, firmware or VM identity to GitHub, chat or a public link.
Do not rely on reading Linux's encrypted home from macOS or the macOS APFS volume
from Linux. A file left only in macOS Downloads is not a completed transfer.

## 4. Optional: collect Machine Lab guest files during this visit

Skip this section if you want messaging first. This is an experimental ARM macOS
KVM lane, with no proven boot or suspend/save result on our M1 Pro Linux host.
Collecting these files reduces another trip but cannot guarantee one will never
be needed. A matching booter/guest compatibility problem may still need macOS.

You need **at least 80 GiB free** on the output volume, plus room for the IPSW.
Use APFS/exFAT; FAT32 cannot hold these files. Linux currently lacks room for a
comfortable restore/build cycle. Prefer an external SSD for this section.

1. If Homebrew is installed, run `brew install macosvm`. Otherwise follow
   [macosvm's official installation instructions](https://github.com/s-u/macosvm)
   or install Homebrew from its official site first. The tool requires macOS 12+.
2. Download the pinned Apple restore image in Terminal (large download; `-C -`
   resumes interruption):

   ```sh
   curl --fail --location --retry 3 -C - 'https://updates.cdn-apple.com/2023FallFCS/fullrestores/042-55833/C0830847-A2F8-458F-B680-967991820931/UniversalMac_13.6_22G120_Restore.ipsw' -o "$HOME/Downloads/UniversalMac_13.6_22G120_Restore.ipsw"
   ```

3. Run `/bin/bash "$HOME/Downloads/MSeries-Mac-Visit/Prepare-Optional-VM.command"`.
   At the first prompt enter the absolute IPSW path; at the second enter a **new**
   folder on your large storage, e.g. `/Volumes/YOUR_DRIVE/MSeries-Ventura-VM`.
   Enter paths without shell quotes or backslash escapes. The helper checks the
   pinned SHA-1 and free space, then invokes macosvm to restore a separate guest.
4. Wait for a successful restore and **Bundle created**. Preserve `disk.img`,
   `aux.img`, `aux.img.trimmed`, `vm.json`, and `AVPBooter.vmapple2.bin` together.
   Do not send this guest identity to Bubbles. Keep the IPSW if space permits.
   If restore fails, save its error in a local text file; do not repeatedly rebuild.
5. Ensure these files are on Linux-readable storage before rebooting. Sparse
   images may consume much more physical storage when copied to exFAT/cloud.

The installed native booter may differ from the published proof. The helper has
been syntax/lint checked on Linux, not executed on macOS. Research pin and limits:
[ARM/Linux macOS experiment](https://github.com/steelbrain/experiment-macOS-arm64-on-asahi-linux-arm64/tree/657bb2423ec8d572b22a97dd33563dbba1680e6a).

## 5. Before leaving macOS

- `mac-hardware.txt` saved successfully and available on cross-OS storage, or
  inside the password-encrypted archive uploaded to your private account.
- Native booter collected if present (optional for messaging).
- Optional guest restore completed and its matching artifacts transferred, or
  explicitly skipped because messaging is the priority.
- iPhone nearby for Apple 2FA after reboot. No login challenge started yet.

Return to Linux and this chat. Open **Bubbles**, choose **Local Mac hardware**,
import/paste the full export, and complete Apple Account login there. Keep the
code/password out of chat. We will verify activation and message delivery then;
this Mac visit does not resolve BlueFerry's separate Bluetooth authorization error.
