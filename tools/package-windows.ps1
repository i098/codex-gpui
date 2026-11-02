# Build the release binary and pack it as an MSIX in OutDir (default dist). The package identity
# declares the `codex:` protocol (windows.protocol), so Windows routes codex:// links to the app
# as argv, which main.rs already reads. The original's MSIX does the same and skips
# setAsDefaultProtocolClient on win32.
# Signing: set CODEX_GPUI_SIGN_PFX (path to a .pfx) and CODEX_GPUI_SIGN_PFX_PASSWORD, and set
# CODEX_GPUI_MSIX_PUBLISHER to the certificate subject (for example "CN=Codex GPUI"). Without
# a PFX the package is unsigned and Windows refuses to install it.
param([string]$OutDir = 'dist')
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
$version = (& bash tools/version.sh).Trim()
$publisher = if ($env:CODEX_GPUI_MSIX_PUBLISHER) { $env:CODEX_GPUI_MSIX_PUBLISHER } else { 'CN=Codex GPUI' }
cargo build --locked --release -p app
if ($LASTEXITCODE) { throw 'cargo build failed' }
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
$stage = Join-Path ([IO.Path]::GetTempPath()) ([guid]::NewGuid())
New-Item -ItemType Directory "$stage/Assets" | Out-Null
Copy-Item "$target/release/codex-gpui.exe" $stage

# Tile logos: the 512x512 PNG inside the macOS icon (icns type ic09), resized.
Add-Type -AssemblyName System.Drawing
$icns = [IO.File]::ReadAllBytes((Resolve-Path 'assets/app/AppIcon.icns'))
$offset = 8
$png = $null
while ($offset -lt $icns.Length) {
    $size = [BitConverter]::ToUInt32(($icns[($offset + 4)..($offset + 7)])[3..0], 0)
    if ([Text.Encoding]::ASCII.GetString($icns, $offset, 4) -eq 'ic09') {
        $png = $icns[($offset + 8)..($offset + $size - 1)]
        break
    }
    $offset += $size
}
if (-not $png) { throw 'AppIcon.icns has no 512x512 PNG' }
$source = [Drawing.Image]::FromStream([IO.MemoryStream]::new([byte[]]$png))
foreach ($logo in @{ 'Square44x44Logo.png' = 44; 'Square150x150Logo.png' = 150; 'StoreLogo.png' = 50 }.GetEnumerator()) {
    $bitmap = [Drawing.Bitmap]::new($source, $logo.Value, $logo.Value)
    $bitmap.Save("$stage/Assets/$($logo.Key)", [Drawing.Imaging.ImageFormat]::Png)
}

@"
<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
         xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
         xmlns:uap3="http://schemas.microsoft.com/appx/manifest/uap/windows10/3"
         xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
         IgnorableNamespaces="uap uap3 rescap">
  <Identity Name="CodexGPUI.App" Publisher="$publisher" Version="$version.0" ProcessorArchitecture="x64" />
  <Properties>
    <DisplayName>Codex GPUI</DisplayName>
    <PublisherDisplayName>Codex GPUI</PublisherDisplayName>
    <Logo>Assets\StoreLogo.png</Logo>
  </Properties>
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.17763.0" MaxVersionTested="10.0.22621.0" />
  </Dependencies>
  <Resources><Resource Language="en-us" /></Resources>
  <Applications>
    <Application Id="App" Executable="codex-gpui.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="Codex GPUI" Description="Native Codex desktop app"
                          BackgroundColor="transparent" Square150x150Logo="Assets\Square150x150Logo.png"
                          Square44x44Logo="Assets\Square44x44Logo.png" />
      <Extensions>
        <uap3:Extension Category="windows.protocol">
          <uap3:Protocol Name="codex" />
        </uap3:Extension>
      </Extensions>
    </Application>
  </Applications>
  <Capabilities><rescap:Capability Name="runFullTrust" /></Capabilities>
</Package>
"@ | Set-Content -Encoding utf8 "$stage/AppxManifest.xml"

New-Item -ItemType Directory -Force $OutDir | Out-Null
$msix = Join-Path (Resolve-Path $OutDir) "codex-gpui-$version-windows-x64.msix"
# makeappx and signtool ship with the Windows SDK; the newest installed version is used.
$sdk = Get-ChildItem "${env:ProgramFiles(x86)}/Windows Kits/10/bin/10.*/x64" -Directory -ErrorAction SilentlyContinue |
    Where-Object { Test-Path "$($_.FullName)/makeappx.exe" } | Sort-Object { [version](Split-Path (Split-Path $_.FullName) -Leaf) } | Select-Object -Last 1
if (-not $sdk) { throw 'the Windows SDK (makeappx.exe) is not installed' }
& "$($sdk.FullName)/makeappx.exe" pack /o /d $stage /p $msix
if ($LASTEXITCODE) { throw 'makeappx failed' }
if ($env:CODEX_GPUI_SIGN_PFX) {
    & "$($sdk.FullName)/signtool.exe" sign /fd SHA256 /f $env:CODEX_GPUI_SIGN_PFX /p $env:CODEX_GPUI_SIGN_PFX_PASSWORD $msix
    if ($LASTEXITCODE) { throw 'signtool failed' }
}
Remove-Item -Recurse -Force $stage
$msix
