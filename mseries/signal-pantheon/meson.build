project(
  'signal-pantheon-client',
  'vala', 'c',
  version: '0.5.1',
  meson_version: '>= 0.59.0',
)

gtk = dependency('gtk4', version: '>= 4.12')
granite = dependency('granite-7', version: '>= 7.7')
cairo = dependency('cairo')
json_glib = dependency('json-glib-1.0')
sqlite = dependency('sqlite3')
secret = dependency('libsecret-1')
cc = meson.get_compiler('c')
gst = dependency('gstreamer-1.0')
gst_app = dependency('gstreamer-app-1.0')
gio_unix = dependency('gio-unix-2.0')
crypto = cc.find_library('crypto', required: true)

executable(
  'signal-pantheon-client',
  [
    'src/main.vala',
    'src/call-controller.vala',
    'src/call-media.vala', 'src/call-video.vala',
    'src/appearance.vala', 'src/message-links.vala',
    'src/message-composer.vala',
    'src/attachment-files.vala',
    'src/main-window.vala',
    'src/signal-backend.vala',
    'src/message-event.vala',
    'src/conversation.vala',
    'src/expiration-service.vala',
    'src/client-store.vala',
    'src/message-commands.vala',
    'src/qr-image.c',
    'src/qrcodegen.c',
    'src/history-store.c',
  ],
  dependencies: [gtk, granite, cairo, json_glib, sqlite, secret, crypto, gst, gst_app, gio_unix],
  include_directories: include_directories('src'),
  install: true,
)

backend_test = executable('backend-test',
  ['tests/backend.vala', 'src/signal-backend.vala', 'src/history-store.c'],
  dependencies: [gtk, dependency('gio-unix-2.0'), json_glib, sqlite, secret, crypto],
  include_directories: include_directories('src'))
test('backend-rpc', find_program('tests/run-backend.sh'),
  args: [meson.current_source_dir() / 'tests/fake-signal-cli.py', backend_test.full_path()],
  timeout: 15)
install_data('data/io.github.sdolbec.SignalPantheonClient.desktop',
  install_dir: get_option('datadir') / 'applications')

message_test = executable('message-event-test',
  ['tests/message-event.vala', 'src/message-event.vala'],
  dependencies: json_glib)
test('message-events', message_test)

history_test = executable('history-test', 'tests/history.c',
  dependencies: [json_glib, sqlite, secret, crypto],
  include_directories: include_directories('src'))
test('encrypted-history', history_test)

conversation_test = executable('conversation-test',
  ['tests/conversation.vala', 'src/conversation.vala', 'src/message-event.vala', 'src/message-commands.vala'],
  dependencies: [json_glib, dependency('gio-2.0')])
test('conversation-and-commands', conversation_test)

ui_test = executable('ui-test',
  ['tests/ui.vala', 'src/call-controller.vala', 'src/call-media.vala', 'src/call-video.vala', 'src/appearance.vala', 'src/message-links.vala', 'src/message-composer.vala', 'src/attachment-files.vala', 'src/main-window.vala', 'src/signal-backend.vala',
   'src/message-event.vala', 'src/conversation.vala', 'src/client-store.vala', 'src/expiration-service.vala',
   'src/message-commands.vala', 'src/history-store.c', 'src/qr-image.c', 'src/qrcodegen.c'],
  vala_args: ['--define=UI_TEST'],
  dependencies: [gtk, granite, cairo, json_glib, sqlite, secret, crypto, gst, gst_app, gio_unix],
  include_directories: include_directories('src'))
test('native-ui', find_program('tests/run-ui.sh'),
  args: [meson.current_source_dir() / 'tests/ui-service.py', ui_test.full_path()],
  timeout: 35)

attachment_test = executable('attachment-test',
  ['tests/attachments.vala', 'src/attachment-files.vala', 'src/message-event.vala'],
  dependencies: [json_glib, dependency('gio-2.0')])
test('attachment-files', attachment_test)

expiration_test = executable('expiration-test',
  ['tests/expiration.vala', 'src/conversation.vala', 'src/message-event.vala'],
  dependencies: [json_glib, dependency('gio-2.0')])
test('disappearing-messages', expiration_test)

call_test = executable('call-test',
  ['tests/calls.vala', 'src/call-controller.vala', 'src/call-media.vala', 'src/call-video.vala', 'src/message-event.vala',
   'src/message-commands.vala', 'src/conversation.vala', 'src/signal-backend.vala', 'src/history-store.c'],
  dependencies: [gtk, dependency('gio-unix-2.0'), json_glib, sqlite, secret, crypto, gst, gst_app, gio_unix],
  include_directories: include_directories('src'))
test('voice-call-controller', find_program('tests/run-backend.sh'),
  args: [meson.current_source_dir() / 'tests/ui-service.py', call_test.full_path()], timeout: 15)

media_bridge_test = executable('media-bridge-test',
  ['tests/media-bridge.vala', 'src/call-media.vala'],
  dependencies: [gst, dependency('gio-unix-2.0')])

install_data('data/signal-pantheon', install_dir: get_option('bindir'), install_mode: 'rwxr-xr-x')
if import('fs').is_dir('runtime')
  install_subdir('runtime', install_dir: get_option('datadir') / 'signal-pantheon')
endif

view_once_test = executable('view-once-test',
  ['tests/view-once.vala', 'src/conversation.vala', 'src/message-event.vala'],
  dependencies: [json_glib, dependency('gio-2.0')])
test('view-once-media', view_once_test)

video_bridge_test = executable('video-bridge-test',
  ['tests/video-bridge.vala', 'src/call-video.vala'],
  vala_args: ['--define=UI_TEST'], dependencies: [gtk, gst, gst_app, gio_unix])
