# UI journey for Safe Toolbox: opens the exe on a chosen monitor, visits every page in dark then light,
# starts each page's read-only work, and records per page: time to settle, visible text, a screenshot,
# memory, and an Axe.Windows accessibility scan (dark pass). Nothing is installed, cleaned or repaired.
# Your settings.json is backed up first and put back at the end.
#
# Run it outside any app container (a normal PowerShell, or a scheduled task):
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\ui\run-ui-tests.ps1 -Exe "...\Safe Toolbox.exe"
param(
    [Parameter(Mandatory)] [string] $Exe,
    [string] $Out = (Join-Path $PSScriptRoot '..\..\artifacts\uitest'),
    [string] $Axe = (Join-Path $PSScriptRoot '..\..\artifacts\tools\axe\AxeWindowsCLI.exe'),
    # Physical pixels: x, y, width, height. The default sits on the laptop screen to the right of the main one.
    [int[]] $Bounds = @(2640, 60, 2300, 1280)
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System; using System.Drawing; using System.Runtime.InteropServices;
public static class Cap {
    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
    public struct RECT { public int L, T, R, B; }
    public static int[] Rect(IntPtr h) { SetThreadDpiAwarenessContext(new IntPtr(-4)); RECT r; GetWindowRect(h, out r); return new[] { r.L, r.T, r.R - r.L, r.B - r.T }; }
    public static void Shot(IntPtr h, string path) {
        int[] r = Rect(h);
        using (var b = new Bitmap(r[2], r[3])) { using (var g = Graphics.FromImage(b)) { IntPtr dc = g.GetHdc(); PrintWindow(h, dc, 2); g.ReleaseHdc(dc); } b.Save(path); }
    }
}
'@

$AE = [System.Windows.Automation.AutomationElement]
$Scope = [System.Windows.Automation.TreeScope]::Descendants
$CT = [System.Windows.Automation.ControlType]
function Cond($prop, $val) { New-Object System.Windows.Automation.PropertyCondition($prop, $val) }
function Find($root, [string]$name, $type) {
    $c = Cond $AE::NameProperty $name
    if ($type) { $c = New-Object System.Windows.Automation.AndCondition($c, (Cond $AE::ControlTypeProperty $type)) }
    $root.FindFirst($Scope, $c)
}
function Texts($root) { @($root.FindAll($Scope, (Cond $AE::ControlTypeProperty $CT::Text)) | ForEach-Object { $_.Current.Name } | Where-Object { $_ }) }
function Busy($root) { @(Texts $root | Where-Object { $_ -match "ing($([char]0x2026)|\.\.\.)\s*$" }).Count -gt 0 }
function Settle($root, [int]$timeout) {
    $sw = [Diagnostics.Stopwatch]::StartNew(); Start-Sleep 3
    while ((Busy $root) -and $sw.Elapsed.TotalSeconds -lt $timeout) { Start-Sleep 2 }
    [math]::Round($sw.Elapsed.TotalSeconds, 1)
}
function Invoke-Named($root, [string]$name) {
    $b = Find $root $name $CT::Button
    if ($b -and $b.Current.IsEnabled) { $b.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke(); return $true }
    $false
}
function Pick($root, [string]$combo, [string]$item) {
    $c = Find $root $combo $CT::ComboBox
    $c.GetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern).Expand(); Start-Sleep -Milliseconds 600
    (Find $root $item $CT::ListItem).GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select(); Start-Sleep 2
}
function Bright([string]$file) {
    $b = New-Object System.Drawing.Bitmap $file
    try { $c = $b.GetPixel([int]($b.Width * 0.6), [int]($b.Height * 0.5)); [int](($c.R + $c.G + $c.B) / 3) } finally { $b.Dispose() }
}
function Go($root, [string]$page) {
    (Find $root $page $CT::ListItem).GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select()
}

if (Get-Process -Name ([IO.Path]::GetFileNameWithoutExtension($Exe)) -ErrorAction SilentlyContinue) { throw 'Safe Toolbox is already open. Close it first.' }
New-Item -ItemType Directory -Force $Out | Out-Null
Get-ChildItem $Out -File -Recurse -ErrorAction SilentlyContinue | Remove-Item -Force
$Root = Join-Path $env:LOCALAPPDATA 'SafeToolbox'
$SettingsFile = Join-Path $Root 'settings.json'
$Backup = Join-Path $Out 'settings.backup.json'
$ErrLog = Join-Path $Root 'error.log'
$errStart = if (Test-Path $ErrLog) { (Get-Item $ErrLog).Length } else { 0 }
$lists = Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'Safe Toolbox\App lists'
$listsBefore = @(if (Test-Path $lists) { Get-ChildItem $lists | ForEach-Object FullName })
if (Test-Path $SettingsFile) { Copy-Item $SettingsFile $Backup -Force }

$result = [ordered]@{ started = (Get-Date).ToString('s'); exe = $Exe; pages = @(); problems = @() }
$proc = $null
try {
    $s = if (Test-Path $SettingsFile) { Get-Content $SettingsFile -Raw | ConvertFrom-Json } else { [pscustomobject]@{} }
    foreach ($kv in @{ windowBounds = $Bounds; windowMaximized = $false; lastPage = 'home'; theme = 'Dark' }.GetEnumerator()) {
        $s | Add-Member -NotePropertyName $kv.Key -NotePropertyValue $kv.Value -Force
    }
    New-Item -ItemType Directory -Force $Root | Out-Null
    $s | ConvertTo-Json -Depth 6 | Set-Content $SettingsFile -Encoding UTF8

    $sw = [Diagnostics.Stopwatch]::StartNew()
    $proc = Start-Process -FilePath $Exe -PassThru
    do { Start-Sleep -Milliseconds 250; $proc.Refresh() } until ($proc.MainWindowHandle -ne 0 -or $sw.Elapsed.TotalSeconds -gt 30)
    if ($proc.MainWindowHandle -eq 0) { throw 'No window within 30 s.' }
    $result.windowSeconds = [math]::Round($sw.Elapsed.TotalSeconds, 1)
    $hwnd = $proc.MainWindowHandle
    $win = $AE::FromHandle($hwnd)
    $result.windowRect = [Cap]::Rect($hwnd)

    $pages = [ordered]@{
        'Home' = @{ wait = 60 }; 'Updates' = @{ wait = 300 }; 'Clean up' = @{ wait = 240 }; 'Big files' = @{ wait = 240 }
        'Health' = @{ wait = 120 }; 'Repair' = @{ wait = 20 }; 'PATH doctor' = @{ wait = 60 }; 'App list' = @{ wait = 180; click = 'Save my app list' }
        'History' = @{ wait = 30 }; 'Settings' = @{ wait = 20 }
    }
    foreach ($theme in 'dark', 'light') {
        if ($theme -eq 'light') { Go $win 'Settings'; Start-Sleep 2; Pick $win 'Theme' 'Light' }
        foreach ($name in $pages.Keys) {
            $p = $pages[$name]; $slug = ($name -replace '\W', '').ToLower()
            Go $win $name
            $t = Settle $win $p.wait
            if ($theme -eq 'dark' -and $p.click) { if (Invoke-Named $win $p.click) { $t += Settle $win $p.wait } else { $result.problems += "$name : '$($p.click)' was missing or disabled" } }
            if ($proc.HasExited) { throw "The app exited on $name ($theme)." }
            $shot = Join-Path $Out "$theme-$slug.png"; [Cap]::Shot($hwnd, $shot)
            $row = [ordered]@{ page = $name; theme = $theme; seconds = $t; stillBusy = (Busy $win); mb = [math]::Round((Get-Process -Id $proc.Id).WorkingSet64 / 1MB) ; shot = $shot }
            if ($theme -eq 'dark') {
                Texts $win | Set-Content (Join-Path $Out "text-$slug.txt") -Encoding UTF8
                if (Test-Path $Axe) {
                    $axeOut = Join-Path $Out "axe-$slug"
                    $log = & $Axe --processId $proc.Id --outputDirectory $axeOut --scanId $slug 2>&1 | Out-String
                    $row.axeExit = $LASTEXITCODE; $row.axe = ($log -split "`n" | Where-Object { $_ -match 'error|found|fail' } | Select-Object -First 3) -join ' | '
                }
            }
            $result.pages += [pscustomobject]$row
        }
    }
    # "Use my Windows setting" must leave the opposite theme and follow Windows straight away.
    $windowsLight = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize').AppsUseLightTheme -eq 1
    $opposite = if ($windowsLight) { 'Dark' } else { 'Light' }
    Go $win 'Settings'; Start-Sleep 2; Pick $win 'Theme' $opposite; [Cap]::Shot($hwnd, (Join-Path $Out 'theme-opposite.png'))
    Pick $win 'Theme' 'Use my Windows setting'; [Cap]::Shot($hwnd, (Join-Path $Out 'theme-system.png'))
    $shade = Bright (Join-Path $Out 'theme-system.png')
    $result.themeCheck = [ordered]@{ windowsLight = $windowsLight; opposite = Bright (Join-Path $Out 'theme-opposite.png'); system = $shade; followsWindows = (($shade -gt 128) -eq $windowsLight) }
    if (-not $result.themeCheck.followsWindows) { $result.problems += 'Use my Windows setting did not follow Windows.' }
    $result.windowRectEnd = [Cap]::Rect($hwnd)
    # Each save is its own folder; only the ones this run made are checked and removed.
    $newLists = @(if (Test-Path $lists) { Get-ChildItem $lists | Where-Object { $listsBefore -notcontains $_.FullName } })
    $result.appListFiles = @($newLists | Get-ChildItem -Recurse -File | ForEach-Object {
        $ok = $true; if ($_.Extension -eq '.json') { try { Get-Content $_.FullName -Raw | ConvertFrom-Json | Out-Null } catch { $ok = $false } }
        [pscustomobject]@{ name = $_.Name; bytes = $_.Length; validJson = $ok }
    })
    $newLists | Remove-Item -Recurse -Force
    $win.GetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern).Close()
    if (-not $proc.WaitForExit(15000)) { $result.problems += 'The window did not close within 15 s.'; $proc.Kill() }
}
catch { $result.problems += "Stopped: $($_.Exception.Message)"; if ($proc -and -not $proc.HasExited) { $proc.Kill() } }
finally {
    if (Test-Path $Backup) { Copy-Item $Backup $SettingsFile -Force }
    if ((Test-Path $ErrLog) -and (Get-Item $ErrLog).Length -gt $errStart) {
        $fs = [IO.File]::OpenRead($ErrLog); $fs.Seek($errStart, 'Begin') | Out-Null
        $result.newErrors = (New-Object IO.StreamReader($fs)).ReadToEnd(); $fs.Dispose()
    }
    $result.finished = (Get-Date).ToString('s')
    $result | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $Out 'results.json') -Encoding UTF8
    'done' | Set-Content (Join-Path $Out 'done.txt')
}
