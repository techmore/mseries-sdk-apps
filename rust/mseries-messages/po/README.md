# Messages translations

The Messages preview and switcher use the app-owned gettext domain
`io.elementary.mseries.MessagesRust`. GTK initializes the process locale before
building the UI. Missing catalogs retain the English source text.

Regenerate the initial static-string template from the repository root:

```sh
python3 rust/mseries-messages/po/update.py
```

The extractor handles literal `tr("...")` calls and rejects unsupported string
escapes. It does not claim coverage of every Rust string. Never translate API
names, thread IDs, desktop IDs, URLs, or message content.

Compile a reviewed language catalog with `msgfmt`, and install it as
`/usr/share/locale/LANGUAGE/LC_MESSAGES/io.elementary.mseries.MessagesRust.mo`.
Catalogs are not yet shipped. Dynamic status/error messages, accessible names,
plural forms, and the shared SDK's English strings still require migration.
This initial template and fallback path do not establish localization parity.
