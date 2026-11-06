using System.IO.Enumeration;
using System.Runtime.InteropServices;

namespace SafeToolbox.Core;

public enum FileKind { Video, DiskImage, Archive, Installer, Audio, AiModel, Other }

public sealed record BigFile(string Path, long Size, DateTime Modified, FileKind Kind)
{
    public string Name => System.IO.Path.GetFileName(Path);
    public string Folder => System.IO.Path.GetDirectoryName(Path) ?? "";
}

/// <summary>
/// Your largest files, the way Windows' "Cleanup recommendations" shows large or unused files, but across your
/// whole profile. Read-only: it never deletes. The page can send a file to the Recycle Bin when you ask, so it
/// can always be restored.
/// </summary>
public static class BigFiles
{
    private static readonly Dictionary<string, FileKind> Kinds = new(StringComparer.OrdinalIgnoreCase)
    {
        [".mp4"] = FileKind.Video, [".mkv"] = FileKind.Video, [".mov"] = FileKind.Video, [".avi"] = FileKind.Video,
        [".webm"] = FileKind.Video, [".m4v"] = FileKind.Video, [".wmv"] = FileKind.Video, [".ts"] = FileKind.Video,
        [".flv"] = FileKind.Video, [".mts"] = FileKind.Video, [".braw"] = FileKind.Video,
        [".iso"] = FileKind.DiskImage, [".img"] = FileKind.DiskImage, [".vhd"] = FileKind.DiskImage, [".vhdx"] = FileKind.DiskImage,
        [".vmdk"] = FileKind.DiskImage, [".wim"] = FileKind.DiskImage, [".esd"] = FileKind.DiskImage,
        [".zip"] = FileKind.Archive, [".7z"] = FileKind.Archive, [".rar"] = FileKind.Archive, [".tar"] = FileKind.Archive,
        [".gz"] = FileKind.Archive, [".tgz"] = FileKind.Archive, [".xz"] = FileKind.Archive, [".bz2"] = FileKind.Archive,
        [".zst"] = FileKind.Archive, [".cab"] = FileKind.Archive,
        [".exe"] = FileKind.Installer, [".msi"] = FileKind.Installer, [".msix"] = FileKind.Installer, [".msixbundle"] = FileKind.Installer,
        [".appx"] = FileKind.Installer, [".appxbundle"] = FileKind.Installer,
        [".wav"] = FileKind.Audio, [".flac"] = FileKind.Audio, [".mp3"] = FileKind.Audio, [".m4a"] = FileKind.Audio,
        [".aac"] = FileKind.Audio, [".ogg"] = FileKind.Audio, [".aiff"] = FileKind.Audio,
        [".gguf"] = FileKind.AiModel, [".safetensors"] = FileKind.AiModel, [".ckpt"] = FileKind.AiModel, [".pt"] = FileKind.AiModel,
        [".pth"] = FileKind.AiModel, [".onnx"] = FileKind.AiModel,
    };

    /// <summary>Folders under the profile that hold app data, caches or phone placeholders, not your files.</summary>
    private static readonly string[] SkipDirNames = ["AppData", "CrossDevice", "node_modules", "$Recycle.Bin", "OneDrive"];

    public static FileKind KindOf(string path) =>
        Kinds.TryGetValue(System.IO.Path.GetExtension(path), out FileKind k) ? k : FileKind.Other;

    public static string KindName(FileKind k) => k switch
    {
        FileKind.Video => "Video",
        FileKind.DiskImage => "Disk image",
        FileKind.Archive => "Archive",
        FileKind.Installer => "Installer",
        FileKind.Audio => "Audio",
        FileKind.AiModel => "AI model",
        _ => "File",
    };

    /// <summary>Your profile folder (Desktop, Documents, Downloads, Videos and the rest), without AppData.</summary>
    public static List<string> DefaultRoots()
    {
        var roots = new List<string> { Environment.GetFolderPath(Environment.SpecialFolder.UserProfile) };
        return roots.Where(Directory.Exists).ToList();
    }

    /// <summary>The <paramref name="limit"/> largest files of at least <paramref name="minBytes"/>, largest first.</summary>
    public static List<BigFile> Find(IEnumerable<string> roots, long minBytes, int limit, CancellationToken ct = default,
        IProgress<string>? progress = null)
    {
        var heap = new PriorityQueue<BigFile, long>();
        var options = new EnumerationOptions
        {
            RecurseSubdirectories = false,
            IgnoreInaccessible = true,
            AttributesToSkip = FileAttributes.ReparsePoint | FileAttributes.Offline | (FileAttributes)0x00400000 /* recall on data access */,
            ReturnSpecialDirectories = false,
        };
        foreach (string root in roots)
        {
            var stack = new Stack<string>();
            stack.Push(root);
            while (stack.Count > 0)
            {
                ct.ThrowIfCancellationRequested();
                string dir = stack.Pop();
                progress?.Report(dir);
                try
                {
                    var subdirs = new FileSystemEnumerable<string>(dir, (ref FileSystemEntry e) => e.ToFullPath(), options)
                    {
                        ShouldIncludePredicate = (ref FileSystemEntry e) => e.IsDirectory && !Skip(e.FileName),
                    };
                    foreach (string d in subdirs)
                    {
                        stack.Push(d);
                    }
                    var files = new FileSystemEnumerable<BigFile>(dir,
                        (ref FileSystemEntry e) => new BigFile(e.ToFullPath(), e.Length, e.LastWriteTimeUtc.LocalDateTime, KindOf(e.FileName.ToString())),
                        options)
                    {
                        ShouldIncludePredicate = (ref FileSystemEntry e) => !e.IsDirectory && e.Length >= minBytes,
                    };
                    foreach (BigFile f in files)
                    {
                        heap.Enqueue(f, f.Size);
                        if (heap.Count > limit)
                        {
                            heap.Dequeue();
                        }
                    }
                }
                catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or DirectoryNotFoundException)
                {
                }
            }
        }
        var result = new List<BigFile>(heap.Count);
        while (heap.Count > 0)
        {
            result.Add(heap.Dequeue());
        }
        result.Reverse();
        return result;
    }

    private static bool Skip(ReadOnlySpan<char> name)
    {
        if (name.Length == 0 || name[0] == '.')
        {
            return true;
        }
        foreach (string s in SkipDirNames)
        {
            if (name.Equals(s, StringComparison.OrdinalIgnoreCase))
            {
                return true;
            }
        }
        return false;
    }

    // ------------------------------------------------------------------ Recycle Bin

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct SHFILEOPSTRUCT
    {
        public IntPtr hwnd;
        public uint wFunc;
        public string pFrom;
        public string? pTo;
        public ushort fFlags;
        public int fAnyOperationsAborted;
        public IntPtr hNameMappings;
        public string? lpszProgressTitle;
    }

    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    private static extern int SHFileOperation(ref SHFILEOPSTRUCT op);

    private const uint FO_DELETE = 3;
    private const ushort FOF_SILENT = 0x0004, FOF_NOCONFIRMATION = 0x0010, FOF_ALLOWUNDO = 0x0040, FOF_NOERRORUI = 0x0400, FOF_WANTNUKEWARNING = 0x4000;

    /// <summary>
    /// Sends a file to the Recycle Bin (never a permanent delete), refusing files inside an installed program or a
    /// PATH folder. Returns null when done, else the reason it was left.
    /// </summary>
    public static string? Recycle(string path, IReadOnlyCollection<string> inUseFolders)
    {
        if (!File.Exists(path))
        {
            return "It isn't there any more.";
        }
        if (inUseFolders.FirstOrDefault(d => SafeDiskCleaner.Core.PathUtil.Under(path, d)) is string owner)
        {
            return $"It is inside {owner}, which holds an installed program or a PATH folder, so it is left alone.";
        }
        var op = new SHFILEOPSTRUCT
        {
            wFunc = FO_DELETE,
            pFrom = path + "\0\0",
            // FOF_WANTNUKEWARNING: if the Recycle Bin can't take it (too big), Windows asks instead of deleting for good.
            fFlags = (ushort)(FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOF_WANTNUKEWARNING),
        };
        int rc = SHFileOperation(ref op);
        if (rc != 0 || op.fAnyOperationsAborted != 0)
        {
            return op.fAnyOperationsAborted != 0 ? "Cancelled." : $"Windows could not move it (code {rc}). It may be open in another app.";
        }
        return File.Exists(path) ? "Windows left it in place." : null;
    }
}
