use gdk_pixbuf::Pixbuf;

const MAX_PHOTO_BYTES: usize = 256 * 1024;
const MAX_PHOTO_DIMENSION: u32 = 2048;
const MAX_JPEG_SEGMENTS: usize = 256;
const MAX_JPEG_FILL_BYTES: usize = 64;

/// Validate and decode an untrusted JPEG/PNG into a bounded, center-cropped avatar.
///
/// Contact photos arrive through an app-facing OS service contract. The service
/// validates them too, but clients repeat the cheap size and header checks before
/// passing bytes to a toolkit decoder. The decoded cache stays in process memory.
pub fn decode_contact_avatar(data: &[u8], max_size: i32) -> Option<Pixbuf> {
    if max_size <= 0 || !valid_contact_photo(data) {
        return None;
    }

    let bytes = gdk_pixbuf::glib::Bytes::from_owned(data.to_vec());
    let stream = gio::MemoryInputStream::from_bytes(&bytes);
    let scaled =
        Pixbuf::from_stream_at_scale(&stream, max_size, max_size, true, None::<&gio::Cancellable>)
            .ok()?;
    let side = scaled.width().min(scaled.height());
    if side <= 0 {
        return None;
    }
    let x = (scaled.width() - side) / 2;
    let y = (scaled.height() - side) / 2;
    Some(scaled.new_subpixbuf(x, y, side, side))
}

fn valid_contact_photo(data: &[u8]) -> bool {
    if data.is_empty() || data.len() > MAX_PHOTO_BYTES {
        return false;
    }

    let dimensions = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        png_dimensions(data)
    } else if data.starts_with(b"\xff\xd8\xff") {
        jpeg_dimensions(data)
    } else {
        None
    };

    dimensions.is_some_and(|(width, height)| {
        width > 0 && height > 0 && width <= MAX_PHOTO_DIMENSION && height <= MAX_PHOTO_DIMENSION
    })
}

fn png_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 24 || &data[12..16] != b"IHDR" {
        return None;
    }
    Some((
        u32::from_be_bytes(data[16..20].try_into().ok()?),
        u32::from_be_bytes(data[20..24].try_into().ok()?),
    ))
}

fn jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let mut position = 2usize;
    let mut segments = 0usize;
    let mut fill_bytes = 0usize;
    while position + 1 < data.len() {
        if data[position] != 0xff {
            return None;
        }
        let marker = data[position + 1];
        if marker == 0xff {
            fill_bytes += 1;
            if fill_bytes > MAX_JPEG_FILL_BYTES {
                return None;
            }
            position += 1;
            continue;
        }
        segments += 1;
        if segments > MAX_JPEG_SEGMENTS {
            return None;
        }
        position += 2;
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        if marker == 0xd9 || marker == 0xda || marker == 0xd8 || position + 2 > data.len() {
            return None;
        }
        let length = usize::from(u16::from_be_bytes([data[position], data[position + 1]]));
        if length < 2 || position + length > data.len() {
            return None;
        }
        if is_jpeg_frame_marker(marker) {
            if length < 7 {
                return None;
            }
            let height = u32::from(u16::from_be_bytes([data[position + 3], data[position + 4]]));
            let width = u32::from(u16::from_be_bytes([data[position + 5], data[position + 6]]));
            return Some((width, height));
        }
        position += length;
    }
    None
}

fn is_jpeg_frame_marker(marker: u8) -> bool {
    matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf)
}
