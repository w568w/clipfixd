use std::{
    collections::{HashMap, HashSet},
    sync::LazyLock,
    time::Duration,
};

use anyhow::anyhow;
use std::io::Read;
use timeout_readwrite::TimeoutReadExt;
use url::Url;
use wayland_clipboard_listener::{WlClipboardPasteStream, WlListenType};
use wl_clipboard_rs::{
    copy::MimeSource,
    paste::{ClipboardType, MimeType, Seat},
};

const MIMETYPE_BLOCK_RECURSIVE: &str = "application/x-clipfixd-block-recursive";
const MIMETYPE_NAUTILUS: &str = "x-special/gnome-copied-files";
const MIMETYPE_TEXT_URI_LIST: &str = "text/uri-list";
const MIMETYPE_QT_IMAGE: &str = "application/x-qt-image";

static X11_CLIPBOARD: LazyLock<x11_clipboard::Clipboard> =
    LazyLock::new(|| x11_clipboard::Clipboard::new().expect("Failed to initialize X11 clipboard"));

fn main() -> anyhow::Result<()> {
    Ok(wayland_clipboard_listener()?)
}

fn wayland_get_content(mime_type: &str) -> anyhow::Result<Vec<u8>> {
    let (pipe, _) = wl_clipboard_rs::paste::get_contents(
        ClipboardType::Regular,
        Seat::Unspecified,
        MimeType::Specific(mime_type),
    )?;
    let mut pipe = pipe.with_timeout(Duration::from_secs(3));
    let mut bytes = vec![];
    match pipe.read_to_end(&mut bytes) {
        Ok(_) => Ok(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
            eprintln!(
                "Timed out when reading Wayland clipboard content for mime type {}",
                mime_type
            );
            Ok(bytes)
        }
        Err(e) => Err(e.into()),
    }
}

fn wayland_get_all_contents<'a>(
    mime_types: impl IntoIterator<Item = impl AsRef<str> + 'a>,
) -> anyhow::Result<HashMap<String, Vec<u8>>> {
    let mut sources = HashMap::new();
    for mime_type in mime_types {
        let bytes = wayland_get_content(mime_type.as_ref());
        if let Ok(bytes) = bytes {
            sources.insert(mime_type.as_ref().to_string(), bytes);
        } else {
            println!(
                "Failed to get Wayland clipboard content for mime type {}: {}",
                mime_type.as_ref(),
                bytes.err().unwrap()
            );
        }
    }
    Ok(sources)
}

#[allow(dead_code)]
fn copy_to_wayland_clipboard(sources: HashMap<String, Vec<u8>>) -> anyhow::Result<()> {
    let opts = wl_clipboard_rs::copy::Options::new();
    opts.copy_multi(
        sources
            .into_iter()
            .map(|(mime_type, data)| MimeSource {
                mime_type: wl_clipboard_rs::copy::MimeType::Specific(mime_type),
                source: wl_clipboard_rs::copy::Source::Bytes(data.into()),
            })
            .collect(),
    )?;
    Ok(())
}

fn copy_to_x11_clipboard(sources: HashMap<String, Vec<u8>>) -> anyhow::Result<()> {
    let mut targets = vec![];
    for (mime_type, data) in sources {
        let atom = X11_CLIPBOARD.getter.get_atom(&mime_type, false)?;
        targets.push((atom, data));
    }
    X11_CLIPBOARD.store_multiple(X11_CLIPBOARD.setter.atoms.clipboard, targets)?;
    Ok(())
}

fn wayland_clipboard_listener() -> anyhow::Result<()> {
    let mut stream = WlClipboardPasteStream::init(WlListenType::ListenOnCopy)?;
    for message in stream.paste_stream().flatten() {
        let all_mime_types: HashSet<String> =
            message.mime_types.iter().map(|s| s.to_string()).collect();
        if all_mime_types.contains(MIMETYPE_BLOCK_RECURSIVE) {
            println!("Detected recursive copy, ignoring to prevent infinite loop.");
            continue;
        }
        println!("Received from Wayland clipboard: {:?}", all_mime_types);

        let mut modified_sources = HashMap::from([(MIMETYPE_BLOCK_RECURSIVE.to_string(), vec![])]);
        // Workaround: Spectacle (Wayland) copy the image as MIMETYPE_QT_IMAGE. QQ, WPS (X11) do not support it very well.
        // They would stuck for seconds when pasting it. So save it to a temp file and convert it to MIMETYPE_TEXT_URI_LIST.
        if all_mime_types.contains(MIMETYPE_QT_IMAGE) {
            let temp_file = tempfile::Builder::new()
                .prefix("clipfixd")
                .suffix(".png")
                .disable_cleanup(true)
                .tempfile()?;
            let bytes = match wayland_get_content(MIMETYPE_QT_IMAGE) {
                Ok(b) => b,
                Err(e)
                    if matches!(
                        e.downcast_ref::<wl_clipboard_rs::paste::Error>(),
                        Some(wl_clipboard_rs::paste::Error::ClipboardEmpty)
                            | Some(wl_clipboard_rs::paste::Error::NoMimeType)
                    ) =>
                {
                    eprintln!(
                        "When getting {}, clipboard has been emptied.",
                        MIMETYPE_QT_IMAGE
                    );
                    continue;
                }
                Err(e) => return Err(e),
            };
            std::fs::write(temp_file.path(), bytes)?;
            let url = Url::from_file_path(temp_file.path())
                .map_err(|_| anyhow!("Failed to convert path to URL"))?;
            modified_sources.insert(
                MIMETYPE_TEXT_URI_LIST.to_string(),
                url.to_string().into_bytes(),
            );
        }
        // Workaround: QQ (X11) copy the image as MIMETYPE_NAUTILUS, but most KDE (Wayland) apps and third-party apps (Wayland) do not support it.
        // So convert it to MIMETYPE_TEXT_URI_LIST for better compatibility.
        if all_mime_types.contains(MIMETYPE_NAUTILUS) {
            let bytes = match wayland_get_content(MIMETYPE_NAUTILUS) {
                Ok(b) => b,
                Err(e)
                    if matches!(
                        e.downcast_ref::<wl_clipboard_rs::paste::Error>(),
                        Some(wl_clipboard_rs::paste::Error::ClipboardEmpty)
                            | Some(wl_clipboard_rs::paste::Error::NoMimeType)
                    ) =>
                {
                    eprintln!(
                        "When getting {}, clipboard has been emptied.",
                        MIMETYPE_NAUTILUS
                    );
                    continue;
                }
                Err(e) => return Err(e),
            };
            let data = String::from_utf8(bytes)?;
            let paths = parse_nautilus_clipboard(&data);
            if let Some(path) = paths.first() {
                modified_sources.insert(
                    MIMETYPE_TEXT_URI_LIST.to_string(),
                    path.to_string().into_bytes(),
                );
            }
        }

        if modified_sources.len() == 1 {
            // No modifications
            continue;
        }
        println!(
            "Modified Wayland clipboard contents: {:?}",
            modified_sources
        );
        let original_sources = wayland_get_all_contents(all_mime_types)?;
        modified_sources.extend(original_sources);
        copy_to_x11_clipboard(modified_sources)?;
    }
    Ok(())
}

fn parse_nautilus_clipboard(data: &str) -> Vec<Url> {
    let mut lines = data.lines();
    let action = lines.next();
    if action != Some("copy") && action != Some("cut") {
        return vec![];
    }
    lines
        .filter_map(|line| {
            if let Ok(url) = Url::parse(line)
                && url.scheme() == "file"
                && url.to_file_path().is_ok()
            {
                Some(url)
            } else {
                None
            }
        })
        .collect()
}
