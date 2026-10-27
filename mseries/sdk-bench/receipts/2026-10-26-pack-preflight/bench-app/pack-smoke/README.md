# pack smoke

Started from the `settings` template (A settings page: a switch, a dropdown, saved values, reset to defaults.)

```bash
msdk check app.ui.json          # structure, types, HIG rules
msdk gate app.ui.json app.gates.json   # acceptance scenarios
msdk-run app.ui.json --watch    # live window; edit the file and it re-renders
msdk describe                   # what you can use; msdk complete / explain for help
```

Change the state, messages, `update` rules and surfaces in `app.ui.json`; add a scenario to `app.gates.json` for every behaviour you add.
