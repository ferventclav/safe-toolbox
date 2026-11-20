//! Your largest files across your profile, the way Windows' "Cleanup recommendations" shows large files. Read-only:
//! it never deletes. The page can send a file to the Recycle Bin when you ask, so it can always be restored.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::time::SystemTime;

use serde::Serialize;

use crate::cleaner::paths::{ATTR_OFFLINE, ATTR_RECALL_ON_DATA_ACCESS, ATTR_REPARSE, under};
use crate::proc::Cancel;
use crate::time::Stamp;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum FileKind {
    Video,
    DiskImage,
    Archive,
    Installer,
    Audio,
    AiModel,
    Other,
}

impl FileKind {
    pub fn name(self) -> &'static str {
        match self {
            FileKind::Video => "Video",
            FileKind::DiskImage => "Disk image",
            FileKind::Archive => "Archive",
            FileKind::Installer => "Installer",
            FileKind::Audio => "Audio",
            FileKind::AiModel => "AI model",
            FileKind::Other => "File",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BigFile {
    pub path: String,
    pub name: String,
    pub folder: String,
    pub size: u64,
    pub modified: Stamp,
    pub kind: FileKind,
    pub kind_name: &'static str,
}

const KINDS: [(FileKind, &[&str]); 6] = [
    (FileKind::Video, &["mp4", "mkv", "mov", "avi", "webm", "m4v", "wmv", "ts", "flv", "mts", "braw"]),
    (FileKind::DiskImage, &["iso", "img", "vhd", "vhdx", "vmdk", "wim", "esd"]),
    (FileKind::Archive, &["zip", "7z", "rar", "tar", "gz", "tgz", "xz", "bz2", "zst", "cab"]),
    (FileKind::Installer, &["exe", "msi", "msix", "msixbundle", "appx", "appxbundle"]),
    (FileKind::Audio, &["wav", "flac", "mp3", "m4a", "aac", "ogg", "aiff"]),
    (FileKind::AiModel, &["gguf", "safetensors", "ckpt", "pt", "pth", "onnx"]),
];

/// Folders under the profile that hold app data, caches or phone placeholders, not your files.
const SKIP_DIRS: [&str; 5] = ["AppData", "CrossDevice", "node_modules", "$Recycle.Bin", "OneDrive"];

pub fn kind_of(path: &str) -> FileKind {
    let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    KINDS.iter().find(|(_, exts)| exts.contains(&ext.as_str())).map_or(FileKind::Other, |(k, _)| *k)
}

/// Your profile folder (Desktop, Documents, Downloads, Videos and the rest); AppData is skipped while walking.
pub fn default_roots() -> Vec<String> {
    let home = crate::tools::home();
    if home.is_dir() { vec![home.to_string_lossy().into_owned()] } else { Vec::new() }
}

fn skip_dir(name: &str) -> bool {
    name.is_empty() || name.starts_with('.') || SKIP_DIRS.iter().any(|s| s.eq_ignore_ascii_case(name))
}

/// The `limit` largest files of at least `min_bytes`, largest first. Links, cloud placeholders and offline files
/// are skipped, so nothing is downloaded just to be measured.
pub fn find(roots: &[String], min_bytes: u64, limit: usize, cancel: Option<&Cancel>, progress: Option<&dyn Fn(&str)>) -> Vec<BigFile> {
    let skip_attrs = ATTR_REPARSE | ATTR_OFFLINE | ATTR_RECALL_ON_DATA_ACCESS;
    let mut heap: BinaryHeap<Reverse<(u64, String, SystemTime)>> = BinaryHeap::new();
    for root in roots {
        let mut stack = vec![std::path::PathBuf::from(root)];
        while let Some(dir) = stack.pop() {
            if cancel.is_some_and(Cancel::is_cancelled) {
                return Vec::new();
            }
            if let Some(p) = progress {
                p(&dir.to_string_lossy());
            }
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let Ok(md) = e.metadata() else { continue };
                if md.file_attributes() & skip_attrs != 0 {
                    continue;
                }
                if md.is_dir() {
                    if !skip_dir(&e.file_name().to_string_lossy()) {
                        stack.push(e.path());
                    }
                } else if md.len() >= min_bytes {
                    heap.push(Reverse((md.len(), e.path().to_string_lossy().into_owned(), md.modified().unwrap_or(SystemTime::UNIX_EPOCH))));
                    if heap.len() > limit {
                        heap.pop();
                    }
                }
            }
        }
    }
    heap.into_sorted_vec()
        .into_iter()
        .map(|Reverse((size, path, modified))| {
            let p = Path::new(&path);
            let kind = kind_of(&path);
            BigFile {
                name: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                folder: p.parent().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                size,
                modified: modified.into(),
                kind,
                kind_name: kind.name(),
                path,
            }
        })
        .collect()
}

/// Sends a file to the Recycle Bin (never a permanent delete), refusing files inside an installed program or a PATH
/// folder. None when done, else the reason it was left.
pub fn recycle(path: &str, in_use_folders: &[String]) -> Option<String> {
    if !Path::new(path).is_file() {
        return Some("It isn't there any more.".into());
    }
    if let Some(owner) = in_use_folders.iter().find(|d| under(path, d)) {
        return Some(format!("It is inside {owner}, which holds an installed program or a PATH folder, so it is left alone."));
    }
    use windows::Win32::UI::Shell::{SHFILEOPSTRUCTW, SHFileOperationW};
    use windows::core::PCWSTR;
    let from: Vec<u16> = path.encode_utf16().chain([0, 0]).collect();
    // ALLOWUNDO | NOCONFIRMATION | SILENT | NOERRORUI | WANTNUKEWARNING: if the bin can't take it, Windows asks
    // instead of deleting for good.
    let mut op = SHFILEOPSTRUCTW { wFunc: 3, pFrom: PCWSTR(from.as_ptr()), fFlags: (0x0040 | 0x0010 | 0x0004 | 0x0400 | 0x4000) as u16, ..Default::default() };
    let rc = unsafe { SHFileOperationW(&mut op) };
    if op.fAnyOperationsAborted.as_bool() {
        return Some("Cancelled.".into());
    }
    if rc != 0 {
        return Some(format!("Windows could not move it (code {rc}). It may be open in another app."));
    }
    Path::new(path).exists().then(|| "Windows left it in place.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_largest_and_skips_app_data_and_dot_folders() {
        let root = std::env::temp_dir().join(format!("st-big-{}", std::process::id()));
        let put = |rel: &str, n: usize| {
            let f = root.join(rel);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, vec![b'x'; n]).unwrap();
        };
        put("Videos/trip.mp4", 5000);
        put("Downloads/setup.exe", 3000);
        put("Downloads/small.txt", 4);
        put("AppData/Local/cache.bin", 9000);
        put(".cache/model.gguf", 9000);
        put("Documents/deep/a/b/c/archive.zip", 4000);
        put("Tools/ffmpeg/ffmpeg.exe", 1);
        let files = find(&[root.to_string_lossy().into_owned()], 1000, 2, None, None);
        assert_eq!(files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["trip.mp4", "archive.zip"]);
        assert_eq!((files[0].kind, files[1].kind), (FileKind::Video, FileKind::Archive));
        let tool = root.join("Tools/ffmpeg/ffmpeg.exe").to_string_lossy().into_owned();
        assert!(recycle(&tool, &[root.join("Tools").to_string_lossy().into_owned()]).is_some());
        assert!(Path::new(&tool).exists());
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!((kind_of("a.MKV"), kind_of("b.iso"), kind_of("c.safetensors"), kind_of("d.docx")), (FileKind::Video, FileKind::DiskImage, FileKind::AiModel, FileKind::Other));
    }
}
