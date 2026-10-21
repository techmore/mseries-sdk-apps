//! How a node looks: its category, colour, icon and the small text helpers shared by every view.

use crate::model::{Kind, Node};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    Folder,
    Image,
    Media,
    Archive,
    DiskImage,
    Package,
    Document,
    Build,
    Other,
    Mount,
}

impl Category {
    pub const LEGEND: [Category; 8] = [
        Category::Folder,
        Category::Image,
        Category::Media,
        Category::Archive,
        Category::DiskImage,
        Category::Package,
        Category::Document,
        Category::Build,
    ];

    /// elementary palette, one step darker than the 500 shades so white text stays readable.
    pub fn rgb(self) -> (f64, f64, f64) {
        let hex = match self {
            Category::Folder => 0x2e6fc9,
            Category::Image => 0xc7366f,
            Category::Media => 0x8a52c8,
            Category::Archive => 0xd9791a,
            Category::DiskImage => 0xb3222a,
            Category::Package => 0x1f9e8a,
            Category::Document => 0x4f9a16,
            Category::Build => 0xa87f12,
            Category::Other => 0x5d6e7b,
            Category::Mount => 0x7d8790,
        };
        (((hex >> 16) & 0xff) as f64 / 255.0, ((hex >> 8) & 0xff) as f64 / 255.0, (hex & 0xff) as f64 / 255.0)
    }

    pub fn label(self) -> &'static str {
        match self {
            Category::Folder => "Folder",
            Category::Image => "Image",
            Category::Media => "Audio and video",
            Category::Archive => "Archive",
            Category::DiskImage => "Disk or VM image",
            Category::Package => "Package or app",
            Category::Document => "Document",
            Category::Build => "Build output",
            Category::Other => "Other file",
            Category::Mount => "Other filesystem",
        }
    }

    /// Short enough for the legend, which must never truncate.
    pub fn short(self) -> &'static str {
        match self {
            Category::Folder => "Folder",
            Category::Image => "Image",
            Category::Media => "Media",
            Category::Archive => "Archive",
            Category::DiskImage => "Disk image",
            Category::Package => "Package",
            Category::Document => "Document",
            Category::Build => "Build",
            Category::Other => "Other",
            Category::Mount => "Other disk",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Category::Folder => "folder",
            Category::Image => "image-x-generic",
            Category::Media => "video-x-generic",
            Category::Archive => "package-x-generic",
            Category::DiskImage => "application-x-cd-image",
            Category::Package => "application-x-executable",
            Category::Document => "x-office-document",
            Category::Build => "text-x-generic",
            Category::Other => "text-x-generic",
            Category::Mount => "drive-harddisk",
        }
    }
}

pub fn category(node: &Node) -> Category {
    match node.kind {
        Kind::Dir => return Category::Folder,
        Kind::Mount => return Category::Mount,
        _ => {}
    }
    let name = node.name.to_lowercase();
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    match ext {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "heif" | "avif" | "tif" | "tiff" | "bmp" | "svg" | "cr2"
        | "cr3" | "nef" | "arw" | "dng" => Category::Image,
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v" | "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "opus" => {
            Category::Media
        }
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "lz4" | "lzma" => Category::Archive,
        "iso" | "img" | "qcow2" | "vmdk" | "vdi" | "vhd" | "vhdx" | "squashfs" | "sqsh" => Category::DiskImage,
        "deb" | "rpm" | "flatpak" | "snap" | "appimage" | "apk" | "so" => Category::Package,
        "pdf" | "doc" | "docx" | "odt" | "ods" | "odp" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" | "csv"
        | "epub" => Category::Document,
        "o" | "a" | "rlib" | "rmeta" | "d" | "pyc" | "class" | "jar" | "wasm" => Category::Build,
        _ => Category::Other,
    }
}

/// "3.2 GB" with the same SI units Files shows.
pub fn size_text(bytes: u64) -> String {
    gtk::glib::format_size(bytes).to_string()
}

pub fn percent_text(part: u64, whole: u64) -> String {
    if whole == 0 {
        return String::new();
    }
    let p = part as f64 / whole as f64 * 100.0;
    if p < 0.1 {
        "<0.1%".into()
    } else if p < 10.0 {
        format!("{p:.1}%")
    } else {
        format!("{p:.0}%")
    }
}

pub fn items_text(items: u64) -> String {
    let n = items.saturating_sub(1);
    match n {
        0 => "empty".into(),
        1 => "1 item".into(),
        _ => format!("{} items", group_digits(n)),
    }
}

pub fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn date_text(unix: i64) -> String {
    gtk::glib::DateTime::from_unix_local(unix)
        .ok()
        .and_then(|d| d.format("%Y-%m-%d").ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> Node {
        Node {
            name: name.into(),
            parent: 0,
            children: vec![],
            size: 1,
            apparent: 1,
            items: 1,
            kind: Kind::File,
            mtime: 0,
            unreadable: false,
        }
    }

    #[test]
    fn categorises_by_extension() {
        assert_eq!(category(&file("Holiday.JPG")), Category::Image);
        assert_eq!(category(&file("backup.tar.zst")), Category::Archive);
        assert_eq!(category(&file("ubuntu.iso")), Category::DiskImage);
        assert_eq!(category(&file("Makefile")), Category::Other);
        assert_eq!(category(&file(".bashrc")), Category::Other);
    }

    #[test]
    fn formats_counts_and_shares() {
        assert_eq!(group_digits(1234567), "1,234,567");
        assert_eq!(group_digits(12), "12");
        assert_eq!(items_text(1), "empty");
        assert_eq!(items_text(2), "1 item");
        assert_eq!(items_text(1001), "1,000 items");
        assert_eq!(percent_text(1, 1000), "0.1%");
        assert_eq!(percent_text(1, 100000), "<0.1%");
        assert_eq!(percent_text(50, 100), "50%");
    }
}
