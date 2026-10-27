# Candidate usage anchor: ARM interaction pass

Hosted ARM SDK run 40415797379 at commit 7fa281f passed the synthetic isolated X11 / private D-Bus anchor test: hover without focus theft, pointer travel, pinning with no duplicate read, outside dismissal, no closed polling, keyboard open, Escape focus restoration and clean exit. The package installation lifecycle also passed. This is a reusable candidate fixture, not an installed status host or owner acceptance. A subsequent probe adds an explicit unpinned pointer-leave check and must be separately validated.
