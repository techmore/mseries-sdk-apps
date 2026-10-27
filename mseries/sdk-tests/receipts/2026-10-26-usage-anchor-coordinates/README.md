# Candidate usage anchor: coordinate diagnostic

Hosted ARM SDK run 40407270110 at commit dc1c147 failed the synthetic hover probe. The accessibility-reported anchor center was (38, 17), and X11 reported the pointer on the root window; no synthetic provider read occurred. This evidence does not establish the cause of the hover failure. The fixture was subsequently changed to export GTK widget bounds for the probe. Native validation of that change remains pending. This receipt contains synthetic data only.
