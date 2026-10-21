"""GTK 4 widget builders for the native Messages window.

Builders take plain dictionaries prepared by messages_model and return widgets.
They hold no application state, so the controller in messages.py decides what
to show and these decide how it looks.
"""
from types import SimpleNamespace
import gettext

import gi
gi.require_version('Gtk', '4.0')
gi.require_version('Gdk', '4.0')
gi.require_version('GdkPixbuf', '2.0')
from gi.repository import Gdk, GdkPixbuf, GLib, Gtk, Pango
from blueferry.message_links import linkify_message
from blueferry.time_display import format_message_timestamp
from message_media import media_links
from messages_model import avatar_tone, conversation_preview, initials, message_parts, short_stamp

_ = gettext.gettext
AVATAR_PX = dict(s=34, m=44, l=54)


def label(text='', css=None, **kwargs):
    widget = Gtk.Label(label=text, **kwargs)
    if css:
        widget.add_css_class(css)
    return widget


def clear(box):
    while box.get_first_child():
        box.remove(box.get_first_child())


def icon_for(*names):
    """First installed icon name, so a missing icon never leaves a blank button."""
    display = Gdk.Display.get_default()
    theme = Gtk.IconTheme.get_for_display(display) if display else None
    for name in names:
        if theme is None or theme.has_icon(name):
            return name
    return names[-1]


def is_unread(thread):
    return bool(thread.get('unread_count') or thread.get('unread'))


def unread_marker(thread):
    """Fixed-width slot left of an avatar holding the unread dot.

    The slot keeps its width when read, so avatars never shift as state changes.
    """
    slot = Gtk.Box(width_request=10, valign=Gtk.Align.CENTER, hexpand=False)
    dot = Gtk.Box(valign=Gtk.Align.CENTER, visible=is_unread(thread), tooltip_text=_('Unread message'))
    dot.add_css_class('ms-unread')
    dot.update_property([Gtk.AccessibleProperty.LABEL], [_('Unread message')])
    slot.append(dot)
    return slot


# --- Avatars --------------------------------------------------------------

def photo_frame(data, px):
    """Round contact photo, decoded in memory at twice the size for HiDPI.

    BlueFerry validates the encoded size and dimensions before returning the
    bytes. The image is scaled so its short side fills the frame, then the
    centre square is kept, so non-square photos are cropped rather than
    squashed. Invalid data returns None so the caller keeps its initials.
    """
    target = px * 2

    def prepared(loader, width, height):
        # Decode at the cover size instead of materialising the full image.
        scale = max(target / max(width, 1), target / max(height, 1))
        loader.set_size(max(target, round(width * scale)), max(target, round(height * scale)))

    try:
        loader = GdkPixbuf.PixbufLoader.new()
        loader.connect('size-prepared', prepared)
        loader.write(data)
        loader.close()
        pixbuf = loader.get_pixbuf()
        if pixbuf is None:
            return None
        left = max(0, (pixbuf.get_width() - target) // 2)
        top = max(0, (pixbuf.get_height() - target) // 2)
        square = pixbuf.new_subpixbuf(left, top, min(target, pixbuf.get_width()), min(target, pixbuf.get_height()))
        texture = Gdk.Texture.new_for_pixbuf(square)
    except (GLib.Error, TypeError, ValueError):
        return None
    # GtkImage with a pixel size reports exactly px, unlike GtkPicture which
    # would ask for the texture's full 2x size.
    image = Gtk.Image.new_from_paintable(texture)
    image.set_pixel_size(px)
    frame = Gtk.Box(overflow=Gtk.Overflow.HIDDEN, halign=Gtk.Align.CENTER, valign=Gtk.Align.CENTER,
                    hexpand=False, vexpand=False, accessible_role=Gtk.AccessibleRole.PRESENTATION)
    frame.set_size_request(px, px)
    frame.add_css_class('ms-photo')
    frame.append(image)
    return frame


def avatar(name, identity, size='m', group=False, data=None):
    """Photo when available, else coloured initials, else a person/people glyph."""
    px = AVATAR_PX[size]
    if data and not group and (photo := photo_frame(data, px)):
        return photo
    # Explicit hexpand=False: the centred label inside must not make the avatar
    # (and every row holding it) grab spare width.
    frame = Gtk.Box(halign=Gtk.Align.CENTER, valign=Gtk.Align.CENTER, hexpand=False, vexpand=False,
                    accessible_role=Gtk.AccessibleRole.PRESENTATION)
    frame.set_size_request(px, px)
    for css in ('ms-avatar', 'ms-avatar-' + size, 'ms-tone-%d' % avatar_tone(identity)):
        frame.add_css_class(css)
    letters = initials(name)
    if group or not letters:
        glyph = icon_for('system-users-symbolic', 'avatar-default-symbolic') if group else icon_for('avatar-default-symbolic')
        icon = Gtk.Image.new_from_icon_name(glyph)
        icon.set_pixel_size(px // 2)
        icon.set_hexpand(True)
        frame.append(icon)
    else:
        frame.append(label(letters, hexpand=True, halign=Gtk.Align.CENTER, valign=Gtk.Align.CENTER))
    return frame


def identity_of(thread):
    """Colour identity: the address for people, the thread key for groups."""
    if not thread.get('is_group') and thread.get('recipients'):
        return thread['recipients'][0]
    return thread.get('key') or thread.get('name', '')


# --- Sidebar --------------------------------------------------------------

def conversation_row(thread, now=None, photo=None):
    """One sidebar row. Returns (row, avatar_slot, avatar) so photos can swap in later."""
    name = thread.get('name') or '?'
    preview, mine = conversation_preview(thread)
    if not preview:
        preview = _('New conversation')
    elif mine:
        preview = _('You: %s') % preview
    last = (thread.get('messages') or [{}])[-1].get('timestamp') or thread.get('last_ts', '')
    stamp = short_stamp(last, now)
    unread = is_unread(thread)

    row = Gtk.ListBoxRow()
    content = Gtk.Box(spacing=12)
    content.add_css_class('ms-row')
    # The unread dot sits left of the avatar (owner's request); both live in one
    # slot so a photo swapped in later lands right after the marker.
    slot = Gtk.Box(spacing=6, valign=Gtk.Align.CENTER, hexpand=False)
    slot.append(unread_marker(thread))
    portrait = avatar(name, identity_of(thread), 'm', thread.get('is_group'), photo)
    slot.append(portrait)
    content.append(slot)

    text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2, hexpand=True, valign=Gtk.Align.CENTER)
    top = Gtk.Box(spacing=8)
    title = label(name, 'ms-contact-name', xalign=0, hexpand=True, ellipsize=Pango.EllipsizeMode.END)
    if unread:
        title.add_css_class('ms-unread-name')
    top.append(title)
    top.append(label(stamp, 'ms-row-time', xalign=1, valign=Gtk.Align.BASELINE))
    snippet = label(preview, 'ms-preview', xalign=0, hexpand=True, wrap=True, lines=2,
                    ellipsize=Pango.EllipsizeMode.END, max_width_chars=26)
    text.append(top)
    text.append(snippet)
    content.append(text)
    row.set_child(content)
    spoken = [name, preview] + ([stamp] if stamp else []) + ([_('Unread')] if unread else [])
    row.update_property([Gtk.AccessibleProperty.LABEL], ['. '.join(spoken)])
    return row, slot, portrait


def favorite_tile(thread, photo=None):
    """Large avatar over a short name for the favourites grid."""
    name = thread.get('name') or '?'
    tile = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6, halign=Gtk.Align.CENTER)
    slot = Gtk.Box(spacing=4, halign=Gtk.Align.CENTER, hexpand=False)
    slot.append(unread_marker(thread))
    portrait = avatar(name, identity_of(thread), 'l', thread.get('is_group'), photo)
    slot.append(portrait)
    tile.append(slot)
    tile.append(label(name, 'ms-favorite-name', xalign=.5, ellipsize=Pango.EllipsizeMode.END, max_width_chars=8))
    return tile, slot, portrait


def empty_state(icon, title, text=''):
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6, halign=Gtk.Align.CENTER, valign=Gtk.Align.CENTER)
    box.add_css_class('ms-empty')
    image = Gtk.Image.new_from_icon_name(icon_for(icon, 'dialog-information-symbolic'))
    image.set_pixel_size(32)
    box.append(image)
    heading = label(title, 'heading', wrap=True, justify=Gtk.Justification.CENTER)
    detail = label(text, 'dim-label', wrap=True, justify=Gtk.Justification.CENTER, visible=bool(text))
    box.append(heading)
    box.append(detail)
    box.image, box.title, box.detail = image, heading, detail
    return box


# --- Conversation ---------------------------------------------------------

def set_markup_safely(widget, body):
    """Show a message with BlueFerry's hardened link markup.

    linkify_message escapes all text and only adds links for URLs it judged
    safe, the same call BlueFerry's own clients make. If GTK ever rejects the
    markup, the visible text would differ from the message, so show it as
    plain text instead of a blank or altered bubble.
    """
    widget.set_markup(linkify_message(body))
    if widget.get_text() != body.replace('\r\n', '\n').replace('\r', '\n'):
        widget.set_text(body)


def message_widget(item, media=None):
    """A bubble (with optional sender name, media cards and caption) for one layout item.

    `media(url, automatic)` returns a preview card for a link in the body; the
    caller owns the network and cache side of that.
    """
    message = item['message']
    outgoing = bool(message.get('outgoing'))
    sending = message.get('_state') == 'sending'
    column = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, halign=Gtk.Align.END if outgoing else Gtk.Align.START)
    column.set_margin_top(8 if item['position'] in ('single', 'first') else 2)
    if item['show_sender']:
        column.append(label(message.get('sender', ''), 'ms-sender', xalign=0, ellipsize=Pango.EllipsizeMode.MIDDLE, max_width_chars=32))
    text, attachment, shortened = message_parts(message)
    when = message.get('display_timestamp') or format_message_timestamp(message.get('timestamp'))
    direction = 'ms-outgoing' if outgoing else 'ms-incoming'
    if text:
        bubble = Gtk.Label(wrap=True, selectable=True, xalign=0, max_width_chars=44,
                           natural_wrap_mode=Gtk.NaturalWrapMode.NONE)
        bubble.set_wrap_mode(Pango.WrapMode.WORD_CHAR)
        set_markup_safely(bubble, text)
        for css in ('ms-bubble', direction, 'ms-' + item['position']):
            bubble.add_css_class(css)
        if sending:
            bubble.add_css_class('ms-sending')
        if when:
            bubble.set_tooltip_text(when)
        column.append(bubble)
    if attachment:
        column.append(attachment_chip(direction, item['position'] if not text else 'single'))
    if media is not None:
        for url in media_links(text):
            column.append(media(url, outgoing))
    if shortened:
        column.append(label(_('Shortened by your iPhone · open it there for the rest'), 'ms-caption', xalign=1 if outgoing else 0))
    caption = _('Sending…') if sending else item['caption']
    if caption:
        column.append(label(caption, 'ms-caption', xalign=1 if outgoing else 0))
    return column


def attachment_chip(direction, position):
    """A bubble standing in for a photo or file the phone's Bluetooth link cannot carry."""
    chip = Gtk.Box(spacing=8, halign=Gtk.Align.END if direction == 'ms-outgoing' else Gtk.Align.START)
    for css in ('ms-chip', 'ms-attachment', direction, 'ms-' + position):
        chip.add_css_class(css)
    chip.append(Gtk.Image.new_from_icon_name('mail-attachment-symbolic'))
    chip.append(label(_('Attachment'), xalign=0))
    chip.set_tooltip_text(_('Your iPhone shares only the text of a message over Bluetooth. Open Messages on the iPhone to see this attachment.'))
    chip.update_property([Gtk.AccessibleProperty.LABEL], [_('Attachment. Open Messages on your iPhone to see it.')])
    return chip


def stamp_widget(text):
    return label(text, 'ms-stamp', halign=Gtk.Align.CENTER)


def render_messages(box, items, media=None):
    clear(box)
    for item in items:
        box.append(stamp_widget(item['text']) if item['kind'] == 'stamp' else message_widget(item, media))


# --- Composer, banner, status ---------------------------------------------

def build_composer():
    """Rounded text field with the send button inside, like a native messenger."""
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
    row = Gtk.Box(spacing=8)
    row.add_css_class('ms-composer')
    field = Gtk.Box(spacing=4, hexpand=True)
    field.add_css_class('ms-field')
    editor = Gtk.TextView(wrap_mode=Gtk.WrapMode.WORD_CHAR, accepts_tab=False, top_margin=7, bottom_margin=7,
                          left_margin=12, right_margin=6)
    editor.update_property([Gtk.AccessibleProperty.LABEL], [_('Message')])
    scroll = Gtk.ScrolledWindow(hexpand=True, propagate_natural_height=True, max_content_height=140,
                                hscrollbar_policy=Gtk.PolicyType.NEVER, child=editor)
    placeholder = label(_('Message'), 'ms-placeholder', xalign=0, halign=Gtk.Align.START, valign=Gtk.Align.START,
                        margin_start=12, margin_top=7, can_target=False,
                        accessible_role=Gtk.AccessibleRole.PRESENTATION)
    overlay = Gtk.Overlay(child=scroll, hexpand=True)
    overlay.add_overlay(placeholder)
    field.append(overlay)

    send = Gtk.Button(valign=Gtk.Align.END, tooltip_text=_('Send Message (Ctrl+Enter)'))
    for css in ('suggested-action', 'circular', 'ms-send'):
        send.add_css_class(css)
    send.update_property([Gtk.AccessibleProperty.LABEL], [_('Send Message')])
    face = Gtk.Stack(transition_type=Gtk.StackTransitionType.CROSSFADE)
    face.add_named(Gtk.Image.new_from_icon_name(icon_for('go-up-symbolic', 'mail-send-symbolic')), 'icon')
    spinner = Gtk.Spinner()
    face.add_named(spinner, 'busy')
    send.set_child(face)
    # Like a native messenger, the send button slides in once there is text.
    send_revealer = Gtk.Revealer(child=send, valign=Gtk.Align.END, transition_type=Gtk.RevealerTransitionType.SLIDE_LEFT)
    field.append(send_revealer)
    row.append(field)
    box.append(row)

    status = label('', 'ms-hint', xalign=0, wrap=True)
    revealer = Gtk.Revealer(child=status, transition_type=Gtk.RevealerTransitionType.SLIDE_UP)
    box.append(revealer)
    return SimpleNamespace(box=box, editor=editor, send=send, send_revealer=send_revealer, face=face, spinner=spinner,
                           placeholder=placeholder, status=status, revealer=revealer)


def build_notice():
    """Inline banner for offline/reconnecting states, with an optional Retry."""
    box = Gtk.Box(spacing=10, visible=False)
    box.add_css_class('ms-notice')
    icon = Gtk.Image.new_from_icon_name(icon_for('dialog-information-symbolic'))
    text = label('', xalign=0, wrap=True, hexpand=True, valign=Gtk.Align.CENTER)
    retry = Gtk.Button(label=_('Retry Connection'), valign=Gtk.Align.CENTER, visible=False)
    retry.update_property([Gtk.AccessibleProperty.DESCRIPTION], [_('Reconnects to your iPhone without changing its pairing')])
    for widget in (icon, text, retry):
        box.append(widget)
    return SimpleNamespace(box=box, icon=icon, text=text, retry=retry)


def build_status():
    """Footer line: coloured dot plus words, so state never relies on colour alone."""
    box = Gtk.Box(spacing=8)
    box.add_css_class('ms-status')
    dot = Gtk.Box(valign=Gtk.Align.CENTER, accessible_role=Gtk.AccessibleRole.PRESENTATION)
    dot.add_css_class('ms-dot')
    text = label('', xalign=0, wrap=True, hexpand=True)
    box.append(dot)
    box.append(text)
    return SimpleNamespace(box=box, dot=dot, text=text)
