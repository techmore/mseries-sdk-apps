project('claude-pantheon-indicator', 'vala', 'c', version: '0.1.0')

wingpanel = dependency('wingpanel')
shared_module(
    'claude-pantheon',
    'src/Indicator.vala',
    dependencies: [
        dependency('glib-2.0'),
        dependency('gobject-2.0'),
        dependency('gtk4'),
        dependency('granite-7'),
        dependency('json-glib-1.0'),
        wingpanel,
    ],
    install: true,
    install_dir: wingpanel.get_variable('indicatorsdir', pkgconfig_define: ['libdir', get_option('prefix') / get_option('libdir')]),
)
