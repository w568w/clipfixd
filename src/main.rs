use std::sync::LazyLock;

use anyhow::anyhow;
use url::Url;
use wayland_clipboard_listener::{WlClipboardPasteStream, WlListenType};
use x11_clipboard::Clipboard;

const MIMETYPE_NAUTILUS: &str = "x-special/gnome-copied-files";
const MIMETYPE_TEXT_URI_LIST: &str = "text/uri-list";

static X11_CLIPBOARD: LazyLock<Clipboard> = LazyLock::new(|| Clipboard::new().expect("Failed to initialize X11 clipboard"));

fn main() -> anyhow::Result<()> {
    let x11_listener_handle = std::thread::spawn(x11_clipboard_listener);
    let wayland_listener_handle = std::thread::spawn(wayland_clipboard_listener);
    x11_listener_handle
        .join()
        .expect("X11 listener thread is broken")?;
    wayland_listener_handle
        .join()
        .expect("Wayland listener thread is broken")?;
    Ok(())
}

fn x11_clipboard_listener() -> anyhow::Result<()> {
    loop {
        X11_CLIPBOARD.load_wait(
            X11_CLIPBOARD.getter.atoms.clipboard,
            X11_CLIPBOARD.getter.atoms.string,
            X11_CLIPBOARD.getter.atoms.property,
        )?;

        let targets = X11_CLIPBOARD.list_target_names(X11_CLIPBOARD.getter.atoms.clipboard, None)?;
        // Workaround: QQ (X11) copy the image as MIMETYPE_NAUTILUS, but most KDE (Wayland) apps and third-party apps (Wayland) do not support it.
        // So convert it to MIMETYPE_TEXT_URI_LIST for better compatibility.
        if targets
            .iter()
            .any(|a| String::from_utf8_lossy(a) == MIMETYPE_NAUTILUS)
        {
            let mime = X11_CLIPBOARD.getter.get_atom(MIMETYPE_NAUTILUS, true)?;
            let data = X11_CLIPBOARD.load(
                X11_CLIPBOARD.getter.atoms.clipboard,
                mime,
                X11_CLIPBOARD.getter.atoms.property,
                None,
            )?;
            let data = str::from_utf8(&data)?;
            let paths = parse_nautilus_clipboard(data);
            let path = paths.first().ok_or(anyhow::anyhow!("No paths found"))?;

            let opts = wl_clipboard_rs::copy::Options::new();
            opts.copy(
                wl_clipboard_rs::copy::Source::Bytes(path.to_string().into_bytes().into()),
                wl_clipboard_rs::copy::MimeType::Specific(MIMETYPE_TEXT_URI_LIST.to_string()),
            )?;
            println!("Copied to Wayland clipboard: {}", path);
        }
    }
}

const MIMETYPE_QT_IMAGE: &str = "application/x-qt-image";
fn wayland_clipboard_listener() -> anyhow::Result<()> {
    let mut stream = WlClipboardPasteStream::init(WlListenType::ListenOnCopy)?;
    for message in stream.paste_stream().flatten() {
        let context = message.context;
        println!("Received from Wayland clipboard: {:?}", context.mime_type);
        // Workaround: Spectacle (Wayland) copy the image as MIMETYPE_QT_IMAGE. QQ, WPS (X11) do not support it very well.
        // They would stuck for seconds when pasting it. So save it to a temp file and convert it to STRING for better  
        // compatibility (most X11 apps accept STRING as file path).
        if context.mime_type == MIMETYPE_QT_IMAGE {
            let temp_file = tempfile::Builder::new().prefix("clipfixd").suffix(".png").tempfile()?;
            std::fs::write(temp_file.path(), context.context)?;
            let url = Url::from_file_path(temp_file.path()).map_err(|_| anyhow!("Failed to convert path to URL"))?;
            X11_CLIPBOARD.store(X11_CLIPBOARD.setter.atoms.clipboard, X11_CLIPBOARD.setter.atoms.string, url.to_string())?;
            println!("Copied to X11 clipboard: {}", url);
        }
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
                return Some(url);
            }
            None
        })
        .collect()
}
