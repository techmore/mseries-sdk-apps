# Usage keyboard probe failures — 2026-10-26

Run 40404146612 on source 34a26fa failed twice. Attempt 1 exited before
accessibility registration because GTK could not open the isolated X display.
Attempt 2 opened the app but did not find a button named `Refresh usage` with
the expected role within the timeout. Neither attempt completed a keyboard
journey. Synthetic artifacts are retained unchanged. The cause of either failure
is not established. A follow-up adds bounded accessibility-tree diagnostics;
it must not treat these attempts as a pass.
