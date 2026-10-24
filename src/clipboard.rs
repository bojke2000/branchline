use crate::outline_format::ClipboardPayload;
use clipboard_rs::{Clipboard, ClipboardContent, ClipboardContext};

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ClipboardError(pub String);

pub trait OutlineClipboard {
    fn read(&mut self) -> Result<ClipboardPayload, ClipboardError>;
    fn write(&mut self, payload: ClipboardPayload) -> Result<(), ClipboardError>;
    fn read_limitation(&self) -> Option<&'static str> {
        None
    }
}

#[derive(Default)]
pub struct NativeClipboard {
    context: Option<ClipboardContext>,
}

impl NativeClipboard {
    fn context(&mut self) -> Result<&ClipboardContext, ClipboardError> {
        if self.context.is_none() {
            self.context = Some(ClipboardContext::new().map_err(|error| {
                ClipboardError(format!("Clipboard acquisition failed: {error}"))
            })?);
        }
        Ok(self.context.as_ref().expect("acquired clipboard context"))
    }
}

#[cfg(target_os = "macos")]
fn native_read() -> Result<ClipboardPayload, ClipboardError> {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeHTML, NSPasteboardTypeString};
    use objc2_foundation::NSUTF8StringEncoding;
    autoreleasepool(|_| {
        let board = NSPasteboard::generalPasteboard();
        let before = board.changeCount();
        let items = board
            .pasteboardItems()
            .ok_or_else(|| ClipboardError("Cannot acquire clipboard items.".into()))?;
        if items.len() != 1 {
            return Err(ClipboardError("Copy one outline item first; an empty or multi-item native clipboard cannot be combined safely.".into()));
        }
        let item = items.objectAtIndex(0);
        let types = item.types();
        let plain_type = unsafe { NSPasteboardTypeString };
        let html_type = unsafe { NSPasteboardTypeHTML };
        let plain = if types.iter().any(|kind| &*kind == plain_type) {
            Some(item.stringForType(plain_type).ok_or_else(|| {
                ClipboardError("Advertised clipboard text could not be acquired.".into())
            })?)
        } else {
            None
        };
        let html = if types.iter().any(|kind| &*kind == html_type) {
            Some(item.stringForType(html_type).ok_or_else(|| {
                ClipboardError("Advertised clipboard HTML could not be acquired.".into())
            })?)
        } else {
            None
        };
        let bytes = plain
            .as_ref()
            .map_or(0, |value| {
                value.lengthOfBytesUsingEncoding(NSUTF8StringEncoding)
            })
            .saturating_add(html.as_ref().map_or(0, |value| {
                value.lengthOfBytesUsingEncoding(NSUTF8StringEncoding)
            }));
        if bytes > crate::document::MAX_FILE_BYTES {
            return Err(ClipboardError("Combined clipboard representations exceed 32 MiB; no Rust text payload was copied.".into()));
        }
        let payload = ClipboardPayload {
            plain: plain.map(|value| value.to_string()),
            html: html.map(|value| value.to_string()),
        };
        if board.changeCount() != before {
            return Err(ClipboardError("Clipboard changed during acquisition. Invoke paste again to capture one coherent outline.".into()));
        }
        payload
            .validate()
            .map_err(|error| ClipboardError(error.to_string()))?;
        Ok(payload)
    })
}

impl OutlineClipboard for NativeClipboard {
    fn read(&mut self) -> Result<ClipboardPayload, ClipboardError> {
        #[cfg(target_os = "macos")]
        {
            native_read()
        }
        #[cfg(not(target_os = "macos"))]
        {
            let context = self.context()?;
            context.available_formats().map_err(|error| {
                ClipboardError(format!("Cannot query clipboard formats: {error}"))
            })?;
            let plain = context.get_text().map_err(|error| {
                ClipboardError(format!("Cannot acquire clipboard text: {error}"))
            })?;
            let payload = ClipboardPayload {
                plain: Some(plain),
                html: None,
            };
            payload
                .validate()
                .map_err(|error| ClipboardError(error.to_string()))?;
            Ok(payload)
        }
    }

    fn read_limitation(&self) -> Option<&'static str> {
        #[cfg(target_os = "macos")]
        {
            None
        }
        #[cfg(not(target_os = "macos"))]
        {
            Some(
                "This platform's coherent rich clipboard acquisition is unverified. Only one plain-text representation was read; rich nesting may be unavailable.",
            )
        }
    }

    fn write(&mut self, payload: ClipboardPayload) -> Result<(), ClipboardError> {
        payload
            .validate()
            .map_err(|error| ClipboardError(error.to_string()))?;
        let (Some(plain), Some(html)) = (&payload.plain, &payload.html) else {
            return Err(ClipboardError(
                "Outline copy requires both HTML and plain text; the clipboard was not changed."
                    .into(),
            ));
        };
        self.context()?.set(vec![ClipboardContent::Html(html.clone()), ClipboardContent::Text(plain.clone())])
            .map_err(|error| ClipboardError(format!("Clipboard publication failed and the previous clipboard may already have been cleared: {error}")))?;
        #[cfg(target_os = "macos")]
        let verified = native_read().map_err(|error| ClipboardError(format!("Clipboard publication could not be verified; it may have partially changed: {error}")))?;
        #[cfg(not(target_os = "macos"))]
        let verified = {
            let context = self.context()?;
            ClipboardPayload {
                plain: Some(context.get_text().map_err(|error| {
                    ClipboardError(format!("Published text verification failed: {error}"))
                })?),
                html: Some(context.get_html().map_err(|error| {
                    ClipboardError(format!("Published HTML verification failed: {error}"))
                })?),
            }
        };
        if verified != payload {
            return Err(ClipboardError("Clipboard publication changed or omitted an intended representation; successful copy could not be confirmed.".into()));
        }
        Ok(())
    }
}
